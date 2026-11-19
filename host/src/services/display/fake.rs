//! Test fakes of Mutter's DisplayConfig and gsd-color's night light on the private session bus of
//! tests/rust-sandbox/run-tests.sh (notes/claude/settings-parity.md §7, amendment 33: fixtures, no interface file).
//! The fake has a connection of its own and is reached by its unique name (`mutter::Endpoints`), so tests run side
//! by side and never own a well-known name. It records every call; its introspection data gives the real argument
//! types, so GDBus refuses a call whose signature differs from mutter's.
use super::mutter;
use crate::services::system::common::variant_json;
use glib::variant::ToVariant;
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const DISPLAY_XML: &str = r#"<node><interface name="org.gnome.Mutter.DisplayConfig">
<method name="GetCurrentState"><arg name="serial" type="u" direction="out"/>
<arg name="monitors" type="a((ssss)a(siiddada{sv})a{sv})" direction="out"/>
<arg name="logical_monitors" type="a(iiduba(ssss)a{sv})" direction="out"/>
<arg name="properties" type="a{sv}" direction="out"/></method>
<method name="ApplyMonitorsConfig"><arg name="serial" type="u" direction="in"/><arg name="method" type="u" direction="in"/>
<arg name="logical_monitors" type="a(iiduba(ssa{sv}))" direction="in"/><arg name="properties" type="a{sv}" direction="in"/></method>
<method name="SetBacklight"><arg name="serial" type="u" direction="in"/><arg name="connector" type="s" direction="in"/>
<arg name="value" type="i" direction="in"/></method>
<property name="Backlight" type="(uaa{sv})" access="read"/>
<property name="ApplyMonitorsConfigAllowed" type="b" access="read"/>
<property name="NightLightSupported" type="b" access="read"/>
<property name="PanelOrientationManaged" type="b" access="read"/>
<signal name="MonitorsChanged"/></interface></node>"#;

const COLOR_XML: &str = r#"<node><interface name="org.gnome.SettingsDaemon.Color">
<method name="NightLightPreview"><arg type="u" name="duration" direction="in"/></method>
<property name="NightLightActive" type="b" access="read"/>
<property name="Temperature" type="u" access="readwrite"/>
<property name="DisabledUntilTomorrow" type="b" access="readwrite"/>
<property name="Sunrise" type="d" access="read"/>
<property name="Sunset" type="d" access="read"/></interface></node>"#;

/// A laptop panel (eDP-1, 2560×1600 at 5/3, VRR, a privacy screen, HDR) left of a 4K monitor (HDMI-1 at 2×,
/// underscanning supported): GetCurrentState's answer in GVariant text.
pub(crate) const TWO_MONITORS: &str = "(uint32 7, [(('eDP-1', 'BOE', '0x0a1c', '0x00000000'), \
[('2560x1600@60.002', 2560, 1600, 60.002, 1.6666666666666667, [1.0, 1.25, 1.5, 1.6666666666666667, 2.0], \
{'is-current': <true>, 'is-preferred': <true>}), \
('2560x1600@120.000', 2560, 1600, 120.0, 1.6666666666666667, [1.0, 1.25, 1.5, 2.0], {'refresh-rate-mode': <'fixed'>}), \
('2560x1600@120.000+vrr', 2560, 1600, 120.0, 1.6666666666666667, [1.0, 1.25, 1.5, 2.0], \
{'refresh-rate-mode': <'variable'>}), \
('1920x1200@60.000', 1920, 1200, 60.0, 1.25, [1.0, 1.25, 1.5], @a{sv} {})], \
{'is-builtin': <true>, 'display-name': <'Ingebouwd beeldscherm'>, 'privacy-screen-state': <(false, false)>, \
'min-refresh-rate': <48>, 'supported-color-modes': <[uint32 0, 1]>, 'color-mode': <uint32 0>}), \
(('HDMI-1', 'DEL', 'DELL U2720Q', 'ABC123'), \
[('3840x2160@60.000', 3840, 2160, 60.0, 2.0, [1.0, 1.25, 1.5, 1.75, 2.0], {'is-current': <true>, 'is-preferred': <true>}), \
('1920x1080@60.000', 1920, 1080, 60.0, 1.0, [1.0, 1.25], @a{sv} {})], \
{'is-builtin': <false>, 'display-name': <'Dell 27\"'>, 'is-underscanning': <false>})], \
[(0, 0, 1.6666666666666667, uint32 0, true, [('eDP-1', 'BOE', '0x0a1c', '0x00000000')], @a{sv} {}), \
(1536, 0, 2.0, uint32 0, false, [('HDMI-1', 'DEL', 'DELL U2720Q', 'ABC123')], @a{sv} {})], \
{'layout-mode': <uint32 1>, 'supports-changing-layout-mode': <false>, 'global-scale-required': <false>})";

pub(crate) const STATE_TYPE: &str = "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})";

/// What the fake answers; tests change it between calls.
pub(crate) struct FakeState {
    pub current_state: String,
    /// (D-Bus error name, message) for ApplyMonitorsConfig, for every method or only `apply_error_method`
    pub apply_error: Option<(String, String)>,
    pub apply_error_method: Option<u32>,
    pub allowed: bool,
    pub night_light_supported: bool,
    /// (serial, connector, min, max, value) of the Backlight property; None: no backlight
    pub backlight: Option<(u32, String, i32, i32, i32)>,
    pub night_active: bool,
    pub disabled_until_tomorrow: bool,
    pub temperature: u32,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            current_state: TWO_MONITORS.into(),
            apply_error: None,
            apply_error_method: None,
            allowed: true,
            night_light_supported: true,
            backlight: Some((3, "eDP-1".into(), 0, 19393, 9696)),
            night_active: true,
            disabled_until_tomorrow: false,
            temperature: 2700,
        }
    }
}

/// A running fake: `name` is its unique bus name, `calls` what it received (method, arguments as JSON).
pub(crate) struct Fake {
    pub name: String,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub state: Arc<Mutex<FakeState>>,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The private session bus of tests/rust-sandbox/run-tests.sh (never the live one under /run/user).
pub(crate) fn private_bus_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

fn record(calls: &Mutex<Vec<(String, Value)>>, method: &str, params: &glib::Variant) {
    calls
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((method.to_owned(), variant_json(params)));
}

impl Fake {
    /// The fake on the private bus; None outside the sandbox (the caller skips).
    pub(crate) fn start(state: FakeState) -> Option<Fake> {
        let address = private_bus_address()?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(state));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (thread_calls, thread_state, thread_stop) =
            (calls.clone(), state.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    gio::Cancellable::NONE,
                )
                .expect("private bus");
                let display = gio::DBusNodeInfo::for_xml(DISPLAY_XML).expect("xml");
                let display = display.lookup_interface(mutter::IFACE).expect("iface");
                let color = gio::DBusNodeInfo::for_xml(COLOR_XML).expect("xml");
                let color = color.lookup_interface(mutter::COLOR_IFACE).expect("iface");
                let (c1, s1, s2) = (thread_calls.clone(), thread_state.clone(), thread_state.clone());
                let display_id = connection
                    .register_object(
                        mutter::PATH,
                        &display,
                        move |_, _, _, _, method, params, invocation| {
                            record(&c1, method, &params);
                            let mut st = s1.lock().unwrap_or_else(|e| e.into_inner());
                            match method {
                                "GetCurrentState" => {
                                    let ty = glib::VariantTy::new(STATE_TYPE).expect("type");
                                    let reply = glib::Variant::parse(Some(ty), &st.current_state)
                                        .expect("fixture");
                                    invocation.return_value(Some(&reply));
                                }
                                "ApplyMonitorsConfig" => {
                                    let method = params.child_value(1).get::<u32>();
                                    match &st.apply_error {
                                        Some((name, message))
                                            if st.apply_error_method.is_none()
                                                || st.apply_error_method == method =>
                                        {
                                            invocation.return_dbus_error(name, message)
                                        }
                                        _ => invocation.return_value(None),
                                    }
                                }
                                "SetBacklight" => {
                                    let (serial, _, value) =
                                        params.get::<(u32, String, i32)>().expect("args");
                                    match st.backlight.as_mut() {
                                        Some(b) if b.0 == serial && (b.2..=b.3).contains(&value) => {
                                            b.4 = value;
                                            b.0 += 1;
                                            invocation.return_value(None);
                                        }
                                        Some(b) if b.0 != serial => invocation.return_dbus_error(
                                            "org.freedesktop.DBus.Error.InvalidArgs",
                                            "Invalid backlight serial",
                                        ),
                                        _ => invocation.return_dbus_error(
                                            "org.freedesktop.DBus.Error.InvalidArgs",
                                            "Invalid backlight value",
                                        ),
                                    }
                                }
                                _ => invocation.return_dbus_error(
                                    "org.freedesktop.DBus.Error.UnknownMethod",
                                    "unknown",
                                ),
                            }
                        },
                        move |_, _, _, _, property| {
                            let st = s2.lock().unwrap_or_else(|e| e.into_inner());
                            match property {
                                "Backlight" => {
                                    let text = match &st.backlight {
                                        Some((serial, connector, min, max, value)) => format!(
                                            "(uint32 {serial}, [{{'connector': <'{connector}'>, 'active': <true>, \
                                             'min': <{min}>, 'max': <{max}>, 'value': <{value}>}}])"
                                        ),
                                        None => "(uint32 1, @aa{sv} [])".into(),
                                    };
                                    glib::Variant::parse(
                                        Some(glib::VariantTy::new("(uaa{sv})").expect("type")),
                                        &text,
                                    )
                                    .expect("backlight")
                                }
                                "ApplyMonitorsConfigAllowed" => st.allowed.to_variant(),
                                "NightLightSupported" => st.night_light_supported.to_variant(),
                                _ => false.to_variant(),
                            }
                        },
                        |_, _, _, _, _, _| false,
                    )
                    .expect("register DisplayConfig");
                let (c2, s3, s4) = (thread_calls.clone(), thread_state.clone(), thread_state.clone());
                let color_id = connection
                    .register_object(
                        mutter::COLOR_PATH,
                        &color,
                        |_, _, _, _, _, _, invocation| invocation.return_value(None),
                        move |_, _, _, _, property| {
                            let st = s3.lock().unwrap_or_else(|e| e.into_inner());
                            match property {
                                "NightLightActive" => st.night_active.to_variant(),
                                "Temperature" => st.temperature.to_variant(),
                                "DisabledUntilTomorrow" => st.disabled_until_tomorrow.to_variant(),
                                "Sunrise" => 6.25f64.to_variant(),
                                _ => 20.5f64.to_variant(),
                            }
                        },
                        move |_, _, _, _, property, value| {
                            record(&c2, &format!("Set {property}"), &value);
                            let mut st = s4.lock().unwrap_or_else(|e| e.into_inner());
                            if property == "DisabledUntilTomorrow"
                                && let Some(on) = value.get::<bool>()
                            {
                                st.disabled_until_tomorrow = on;
                                st.night_active = !on;
                                return true;
                            }
                            false
                        },
                    )
                    .expect("register Color");
                ready
                    .send((connection.unique_name().expect("name").to_string(), context.clone()))
                    .expect("ready");
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                let _ = connection.unregister_object(display_id);
                let _ = connection.unregister_object(color_id);
                let _ = connection.close_sync(gio::Cancellable::NONE);
            });
        });
        let (name, context) = started.recv().ok()?;
        Some(Fake {
            name,
            calls,
            state,
            stop,
            context,
            thread: Some(thread),
        })
    }

    /// The endpoints of this fake (both services on its one connection).
    pub(crate) fn endpoints(&self) -> mutter::Endpoints {
        mutter::Endpoints {
            mutter: self.name.clone(),
            color: self.name.clone(),
        }
    }

    /// The calls of one method so far.
    pub(crate) fn calls_of(&self, method: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
