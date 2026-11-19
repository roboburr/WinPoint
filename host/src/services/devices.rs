//! `devices.*` (batch B, notes/claude/settings-parity.md §5.3): the Bluetooth adapter as GNOME Settings shows it
//! (its name, discoverable while the page shows, Send Files), the batteries of connected devices (UPower, GNOME's
//! Power › Connected Devices), Thunderbolt / USB4 (bolt, GNOME's Privacy › Thunderbolt) and colour management
//! (colord, colord-session's ColorHelper and Mutter's calibration session, GNOME's Color panel: devices/color.rs).
//!
//! Reads go to the daemons on the caller's thread. Every dangerous method (`METHODS`) checks its arguments first and
//! under `ctx.dry_run` answers `{ok, dryRun}` without calling anything. bolt checks polkit itself
//! (`org.freedesktop.bolt.manage`, `.enroll`, `.authorize`: auth_admin_keep, §5.8), so its writes go through
//! `call_interactive` and GNOME Shell's password dialog; so does colord (modify-device, install-system-wide). A
//! watcher thread with its own MainContext follows UPower, bolt, the BlueZ adapter and colord and emits
//! `devices.changed {what}` (`"batteries"`, `"thunderbolt"`, `"bluetooth"`, `"color"`) once per burst: the first
//! signal of a quiet kind arms a one-shot 250 ms source, so an idle watcher sleeps in its main loop. A discoverable
//! lease that is not renewed ends by a one-shot source at its end (devices/bluetooth.rs). colord-session's
//! calibration signals (session bus) become `devices.calibrate {state, ...}` at once (devices/color.rs
//! `calibrate_event`); its UpdateGamma ramps go to Mutter's calibration session (`apply_gamma`).
//! The service starts on its first call (`LazyService`, services/mod.rs).
mod batteries;
mod bluetooth;
mod color;
#[cfg(test)]
pub(crate) mod fake_bus;
mod thunderbolt;

use super::ServiceContext;
use super::system::common::{session, system};
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// How long the watcher gathers the signals of a kind before it emits `devices.changed` once.
const EMIT_DELAY: Duration = Duration::from_millis(250);

/// The methods (name, dangerous), exactly as host/src/api-methods/devices.json lists them (a test compares).
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("devices.batteries", false),
    ("devices.bluetooth.adapter", false),
    ("devices.bluetooth.sendFiles", false),
    ("devices.bluetooth.setDiscoverable", true),
    ("devices.color.addProfile", true),
    ("devices.color.calibrate.cancel", true),
    ("devices.color.calibrate.resume", true),
    ("devices.color.calibrate.start", true),
    ("devices.color.get", false),
    ("devices.color.importProfile", true),
    ("devices.color.installSystemWide", true),
    ("devices.color.makeDefault", true),
    ("devices.color.removeProfile", true),
    ("devices.color.setEnabled", true),
    ("devices.thunderbolt.authorize", true),
    ("devices.thunderbolt.enroll", true),
    ("devices.thunderbolt.forget", true),
    ("devices.thunderbolt.get", false),
    ("devices.thunderbolt.setAuthMode", true),
];

/// Where the daemons are: the system bus and their own names, or (tests) the private session bus and a fake's
/// unique name.
#[derive(Clone)]
pub(crate) struct Endpoints {
    /// The system bus (true) or the session bus (the tests' fake).
    pub system_bus: bool,
    pub bluez: String,
    pub upower: String,
    pub bolt: String,
    pub colord: String,
    /// colord-session (org.freedesktop.ColorHelper), always on the session bus.
    pub color_helper: String,
    /// Mutter's colour manager (org.gnome.Mutter.ColorManager), on the session bus: the calibration's gamma.
    pub mutter: String,
    /// Where imported ICC profiles go (`$XDG_DATA_HOME/icc`, cd_client_import_profile).
    pub icc_dir: std::path::PathBuf,
}

impl Endpoints {
    fn real() -> Self {
        Self {
            system_bus: true,
            bluez: "org.bluez".into(),
            upower: "org.freedesktop.UPower".into(),
            bolt: "org.freedesktop.bolt".into(),
            colord: "org.freedesktop.ColorManager".into(),
            color_helper: "org.freedesktop.ColorHelper".into(),
            mutter: color::MUTTER.into(),
            icc_dir: glib::user_data_dir().join("icc"),
        }
    }

    /// Every daemon behind one name on the session bus (the tests' fake).
    #[cfg(test)]
    pub(crate) fn session(name: &str) -> Self {
        Self {
            system_bus: false,
            bluez: name.into(),
            upower: name.into(),
            bolt: name.into(),
            colord: name.into(),
            color_helper: name.into(),
            mutter: name.into(),
            icc_dir: std::env::temp_dir().join(format!("win11-icc-{}", crate::events::nonce())),
        }
    }

    fn bus(&self) -> Result<gio::DBusConnection, String> {
        if self.system_bus { system() } else { session() }
    }

    /// colord-session's bus (the session bus, also in the tests).
    fn helper_bus(&self) -> Result<gio::DBusConnection, String> {
        session()
    }
}

/// An argument that does not fit (`bad_args` with a key of devices.tsv).
pub(super) fn bad(lang: Lang, key: &str) -> ApiError {
    ApiError::tr("bad_args", lang, key, crate::i18n::NO_PARAMS)
}

/// The discoverable lease and whether a one-shot source waits for its end (both under one lock, so a renewal and
/// the source never miss each other).
#[derive(Default)]
struct LeaseState {
    lease: bluetooth::Lease,
    armed: bool,
}

pub(crate) struct Devices {
    ctx: Arc<ServiceContext>,
    ends: Endpoints,
    lease: Mutex<LeaseState>,
    /// How long a lease lasts without a renewal (bluetooth::LEASE; shorter in the tests).
    lease_for: Duration,
    /// Mutter's calibration session while a display calibration runs (devices/color.rs).
    gamma: Mutex<Option<color::Gamma>>,
    stopped: Arc<AtomicBool>,
    context: Mutex<Option<glib::MainContext>>,
    me: Weak<Devices>,
}

impl Devices {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let devices = Self::with(ctx, Endpoints::real());
        devices.watch();
        Ok(devices)
    }

    fn with(ctx: Arc<ServiceContext>, ends: Endpoints) -> Arc<Self> {
        Self::with_lease(ctx, ends, bluetooth::LEASE)
    }

    fn with_lease(ctx: Arc<ServiceContext>, ends: Endpoints, lease_for: Duration) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            ctx,
            ends,
            lease: Mutex::new(LeaseState::default()),
            lease_for,
            gamma: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
            context: Mutex::new(None),
            me: me.clone(),
        })
    }

    fn changed(&self, what: &str) {
        self.ctx.emit("devices.changed", json!({ "what": what }));
    }

    /// Check a method's arguments (also in a dry run, so the page hears about a bad one).
    fn check(method: &str, args: &Value, lang: Lang) -> Result<(), ApiError> {
        match method {
            "devices.bluetooth.setDiscoverable" => bluetooth::on_arg(&args["on"], lang).map(drop),
            "devices.bluetooth.sendFiles" => bluetooth::send_args(args, lang).map(drop),
            "devices.thunderbolt.setAuthMode" => {
                thunderbolt::mode_arg(&args["mode"], lang).map(drop)
            }
            "devices.thunderbolt.enroll" => {
                thunderbolt::uid_arg(&args["uid"], lang)?;
                thunderbolt::policy_arg(&args["policy"], lang).map(drop)
            }
            "devices.thunderbolt.authorize" | "devices.thunderbolt.forget" => {
                thunderbolt::uid_arg(&args["uid"], lang).map(drop)
            }
            "devices.color.setEnabled" => {
                color::path_arg(&args["device"], lang)?;
                bluetooth::on_arg(&args["on"], lang).map(drop)
            }
            "devices.color.makeDefault"
            | "devices.color.addProfile"
            | "devices.color.removeProfile" => {
                color::path_arg(&args["device"], lang)?;
                color::path_arg(&args["profile"], lang).map(drop)
            }
            "devices.color.installSystemWide" => {
                color::path_arg(&args["profile"], lang)?;
                if args["device"].is_null() {
                    Ok(())
                } else {
                    color::path_arg(&args["device"], lang).map(drop)
                }
            }
            "devices.color.importProfile" => {
                color::path_arg(&args["device"], lang)?;
                match args["path"].as_str() {
                    Some(p) if p.starts_with('/') && !p.contains('\0') && p.len() <= 4096 => Ok(()),
                    _ => Err(bad(lang, "devices.badArgs")),
                }
            }
            "devices.color.calibrate.start" => Self::calibrate_args(args, lang).map(drop),
            _ => Ok(()),
        }
    }

    /// The calibration's choices: GNOME's quality, display type and white point, a profile title (1-100
    /// characters), the device and optionally the sensor.
    fn calibrate_args(
        args: &Value,
        lang: Lang,
    ) -> Result<(String, Option<String>, (u32, u32, u32, String)), ApiError> {
        let device = color::path_arg(&args["device"], lang)?;
        let sensor = match &args["sensor"] {
            Value::Null => None,
            v => Some(color::path_arg(v, lang)?),
        };
        let pick = |v: &Value, allowed: &[u32]| {
            v.as_u64()
                .map(|n| n as u32)
                .filter(|n| allowed.contains(n))
                .ok_or_else(|| bad(lang, "devices.badArgs"))
        };
        let quality = pick(&args["quality"], color::QUALITIES)?;
        let kind = pick(&args["kind"], color::KINDS)?;
        let whitepoint = pick(&args["whitepoint"], color::WHITEPOINTS)?;
        let title = args["title"]
            .as_str()
            .map(str::trim)
            .filter(|t| {
                !t.is_empty() && t.chars().count() <= 100 && !t.chars().any(char::is_control)
            })
            .ok_or_else(|| bad(lang, "devices.badArgs"))?
            .to_owned();
        Ok((device, sensor, (quality, kind, whitepoint, title)))
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let dangerous = METHODS.iter().find(|(name, _)| *name == method)?.1;
        let lang = self.ctx.lang();
        if let Err(error) = Self::check(method, args, lang) {
            return Some(Err(error));
        }
        if dangerous && self.ctx.dry_run {
            return Some(Ok(json!({"ok": true, "dryRun": true})));
        }
        Some(self.run(method, args, lang))
    }

    fn run(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let bus = self.ends.bus();
        match method {
            "devices.bluetooth.adapter" => Ok(match &bus {
                Ok(bus) => bluetooth::adapter(bus, &self.ends.bluez),
                Err(_) => bluetooth::absent(),
            }),
            "devices.bluetooth.setDiscoverable" => {
                let on = bluetooth::on_arg(&args["on"], lang)?;
                let bus = bus.map_err(|_| bluetooth::no_bluetooth(lang))?;
                let mut st = self.lease.lock().unwrap_or_else(|e| e.into_inner());
                let value = bluetooth::set_discoverable(
                    &bus,
                    &self.ends.bluez,
                    &mut st.lease,
                    on,
                    self.lease_for,
                    lang,
                )?;
                if !st.armed
                    && let Some(until) = st.lease.until()
                {
                    st.armed = self.arm_expiry(until);
                }
                drop(st);
                self.changed("bluetooth");
                Ok(value)
            }
            "devices.bluetooth.sendFiles" => {
                let (address, name) = bluetooth::send_args(args, lang)?;
                bluetooth::send_files(
                    crate::tools::system_tool("bluetooth-sendto"),
                    &address,
                    &name,
                    lang,
                )
            }
            "devices.batteries" => Ok(match &bus {
                Ok(bus) => batteries::get(bus, &self.ends.upower),
                Err(_) => batteries::absent(),
            }),
            "devices.thunderbolt.get" => Ok(match &bus {
                Ok(bus) => thunderbolt::get(bus, &self.ends.bolt),
                Err(_) => thunderbolt::absent(),
            }),
            "devices.color.get" => Ok(match &bus {
                Ok(bus) => color::get(bus, &self.ends.colord),
                Err(_) => color::absent(),
            }),
            m if m.starts_with("devices.color.") => {
                let bus = bus.map_err(|_| color::unavailable(lang))?;
                let value = self.color(&bus, m, args, lang)?;
                self.changed("color");
                Ok(value)
            }
            _ => {
                let bus = bus.map_err(|_| thunderbolt::unavailable(lang))?;
                let name = self.ends.bolt.as_str();
                let value = match method {
                    "devices.thunderbolt.setAuthMode" => {
                        let mode = thunderbolt::mode_arg(&args["mode"], lang)?;
                        thunderbolt::set_auth_mode(&bus, name, mode, lang)
                    }
                    "devices.thunderbolt.enroll" => {
                        let uid = thunderbolt::uid_arg(&args["uid"], lang)?;
                        let policy = thunderbolt::policy_arg(&args["policy"], lang)?;
                        thunderbolt::enroll(&bus, name, &uid, policy, lang)
                    }
                    "devices.thunderbolt.authorize" => {
                        let uid = thunderbolt::uid_arg(&args["uid"], lang)?;
                        thunderbolt::authorize(&bus, name, &uid, lang)
                    }
                    "devices.thunderbolt.forget" => {
                        let uid = thunderbolt::uid_arg(&args["uid"], lang)?;
                        thunderbolt::forget(&bus, name, &uid, lang)
                    }
                    _ => unreachable!("a method of METHODS"),
                }?;
                self.changed("thunderbolt");
                Ok(value)
            }
        }
    }

    /// The colour writes (devices/color.rs).
    fn color(
        &self,
        bus: &gio::DBusConnection,
        method: &str,
        args: &Value,
        lang: Lang,
    ) -> ApiResult {
        let name = self.ends.colord.as_str();
        let path = |key: &str| color::path_arg(&args[key], lang);
        match method {
            "devices.color.setEnabled" => {
                color::set_enabled(bus, name, &path("device")?, args["on"] == true, lang)
            }
            "devices.color.makeDefault" => {
                color::make_default(bus, name, &path("device")?, &path("profile")?, lang)
            }
            "devices.color.addProfile" => {
                color::add_profile(bus, name, &path("device")?, &path("profile")?, lang)
            }
            "devices.color.removeProfile" => {
                color::remove_profile(bus, name, &path("device")?, &path("profile")?, lang)
            }
            "devices.color.installSystemWide" => {
                let device = if args["device"].is_null() {
                    None
                } else {
                    Some(path("device")?)
                };
                color::install_system_wide(bus, name, &path("profile")?, device.as_deref(), lang)
            }
            "devices.color.importProfile" => color::import_profile(
                bus,
                name,
                &path("device")?,
                std::path::Path::new(args["path"].as_str().unwrap_or("")),
                &self.ends.icc_dir,
                lang,
            ),
            "devices.color.calibrate.start" => {
                let (device, sensor, (quality, kind, whitepoint, title)) =
                    Self::calibrate_args(args, lang)?;
                let session = self
                    .ends
                    .helper_bus()
                    .map_err(|_| color::unavailable(lang))?;
                // GNOME 50's cc-color-calibrate.c: Mutter's calibration session for the display first (its gamma
                // takes colord-session's ramps), then colord-session's Start
                let gamma =
                    color::gamma_session(bus, name, &session, &self.ends.mutter, &device, lang)?;
                self.end_gamma();
                *self.gamma.lock().unwrap_or_else(|e| e.into_inner()) = Some(gamma);
                let started = color::calibrate_start(
                    bus,
                    name,
                    &session,
                    &self.ends.color_helper,
                    &device,
                    sensor.as_deref(),
                    (quality, kind, whitepoint, &title),
                    lang,
                );
                if started.is_err() {
                    self.end_gamma();
                }
                started
            }
            "devices.color.calibrate.cancel" | "devices.color.calibrate.resume" => {
                let session = self
                    .ends
                    .helper_bus()
                    .map_err(|_| color::unavailable(lang))?;
                let member = if method.ends_with("cancel") {
                    "Cancel"
                } else {
                    "Resume"
                };
                let answer =
                    color::helper_call(&session, &self.ends.color_helper, member, None, lang);
                if member == "Cancel" {
                    // colord-session answers a cancel with Finished too; the session ends here already
                    self.end_gamma();
                }
                answer
            }
            _ => Err(bad(lang, "devices.badArgs")),
        }
    }

    fn calibrate_event(&self, data: Value) {
        self.ctx.emit("devices.calibrate", data);
    }

    /// colord-session's UpdateGamma (a(ddd)): the ramp into the display's gamma LUT through Mutter's calibration
    /// session, as cc-color-calibrate.c (a failure is only a warning there too).
    fn apply_gamma(&self, params: &glib::Variant) {
        let Some(ramp) = color::gamma_ramp(params) else {
            eprintln!("win11-host: devices: no data in the CLUT array");
            return;
        };
        let held = self.gamma.lock().unwrap_or_else(|e| e.into_inner());
        let Some(gamma) = held.as_ref() else { return };
        let Ok(session) = self.ends.helper_bus() else {
            return;
        };
        if let Err(error) = color::set_gamma(&session, &self.ends.mutter, gamma, &ramp) {
            eprintln!("win11-host: devices: failed to update gamma: {error}");
        }
    }

    /// The calibration is over (Finished, a cancel, a new start, the host stopping): Mutter's session stops, which
    /// gives the display its profile's curves back.
    fn end_gamma(&self) {
        let Some(gamma) = self.gamma.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        if let Ok(session) = self.ends.helper_bus() {
            color::stop_gamma(&session, &self.ends.mutter, &gamma);
        }
    }

    /// A one-shot source on the watcher's context at the lease's end (`lease_tick`); false without a watcher.
    fn arm_expiry(&self, at: Instant) -> bool {
        let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return false;
        };
        let weak = self.me.clone();
        glib::timeout_source_new(
            at.saturating_duration_since(Instant::now()),
            None,
            glib::Priority::DEFAULT,
            move || {
                if let Some(me) = weak.upgrade() {
                    me.lease_tick();
                }
                glib::ControlFlow::Break
            },
        )
        .attach(Some(&context));
        true
    }

    /// The lease's end came: it ends when it was not renewed meanwhile, else the source waits for the new end.
    fn lease_tick(&self) {
        let mut st = self.lease.lock().unwrap_or_else(|e| e.into_inner());
        st.armed = false;
        if st.lease.expired() {
            match self.ends.bus() {
                Ok(bus) => bluetooth::end_lease(&bus, &self.ends.bluez, &mut st.lease),
                Err(_) => st.lease = bluetooth::Lease::default(),
            }
            drop(st);
            self.changed("bluetooth");
        } else if let Some(until) = st.lease.until() {
            st.armed = self.arm_expiry(until);
        }
    }

    /// The watcher (module docs): the first signal of UPower, bolt, BlueZ or colord after a quiet spell marks its
    /// kind dirty and arms a one-shot source that emits every dirty kind once, 250 ms later. Signal callbacks and
    /// sources all run on this thread, so the dirty set needs no more care than its lock.
    fn watch(self: &Arc<Self>) {
        let context = glib::MainContext::new();
        *self.context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
        let weak = Arc::downgrade(self);
        let stopped = self.stopped.clone();
        let ends = self.ends.clone();
        std::thread::spawn(move || {
            let _ = context.with_thread_default(|| {
                let dirty: Arc<Mutex<BTreeSet<&'static str>>> = Arc::default();
                let bus = ends.bus().ok();
                let mut subs = Vec::new();
                if let Some(bus) = bus.as_ref() {
                    // UPower and bolt: every signal (PropertiesChanged, DeviceAdded, DeviceRemoved); BlueZ: the
                    // adapter's properties only (a discovery changes the devices' RSSI many times a second)
                    let props = Some("org.freedesktop.DBus.Properties");
                    for (name, iface, member, arg0, what) in [
                        (ends.upower.clone(), None, None, None, "batteries"),
                        (ends.bolt.clone(), None, None, None, "thunderbolt"),
                        // colord: DeviceAdded/Removed/Changed, ProfileAdded/..., SensorAdded/Removed and the
                        // objects' own Changed
                        (
                            ends.colord.clone(),
                            None,
                            Some("DeviceAdded"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("DeviceRemoved"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("DeviceChanged"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("ProfileAdded"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("ProfileRemoved"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("ProfileChanged"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("SensorAdded"),
                            None,
                            "color",
                        ),
                        (
                            ends.colord.clone(),
                            None,
                            Some("SensorRemoved"),
                            None,
                            "color",
                        ),
                        (
                            ends.bluez.clone(),
                            props,
                            Some("PropertiesChanged"),
                            Some("org.bluez.Adapter1"),
                            "bluetooth",
                        ),
                    ] {
                        let (dirty, weak, context) = (dirty.clone(), weak.clone(), context.clone());
                        subs.push(bus.signal_subscribe(
                            Some(&name),
                            iface,
                            member,
                            None,
                            arg0,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, _, _, _, _| {
                                let first = {
                                    let mut set = dirty.lock().unwrap_or_else(|e| e.into_inner());
                                    let first = set.is_empty();
                                    set.insert(what);
                                    first
                                };
                                if first {
                                    arm_emit(&context, &dirty, &weak);
                                }
                            },
                        ));
                    }
                }
                // colord-session's calibration (session bus): every signal of ColorHelper.Display and its Progress
                let helper_bus = ends.helper_bus().ok();
                let mut helper_subs = Vec::new();
                if let Some(hb) = helper_bus.as_ref() {
                    let w = weak.clone();
                    helper_subs.push(hb.signal_subscribe(
                        Some(&ends.color_helper),
                        Some(color::HELPER_DISPLAY),
                        None,
                        Some(color::HELPER_PATH),
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, member, params| {
                            let Some(me) = w.upgrade() else { return };
                            match member {
                                "UpdateGamma" => me.apply_gamma(params),
                                "Finished" => me.end_gamma(),
                                _ => {}
                            }
                            if let Some(data) = color::calibrate_event(member, params) {
                                me.calibrate_event(data);
                            }
                        },
                    ));
                    let w = weak.clone();
                    helper_subs.push(hb.signal_subscribe(
                        Some(&ends.color_helper),
                        Some("org.freedesktop.DBus.Properties"),
                        Some("PropertiesChanged"),
                        Some(color::HELPER_PATH),
                        Some(color::HELPER_DISPLAY),
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, params| {
                            let changed = crate::services::system::common::variant_json(
                                &params.child_value(1),
                            );
                            if let (Some(me), Some(progress)) =
                                (w.upgrade(), changed.get("Progress"))
                            {
                                me.calibrate_event(
                                    json!({"state": "progress", "progress": progress}),
                                );
                            }
                        },
                    ));
                }
                // no source of its own: the loop sleeps until a signal, a one-shot source or shutdown's wakeup
                while !stopped.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                if let Some(bus) = bus {
                    for sub in subs {
                        bus.signal_unsubscribe(sub);
                    }
                }
                if let Some(hb) = helper_bus {
                    for sub in helper_subs {
                        hb.signal_unsubscribe(sub);
                    }
                }
            });
        });
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            context.wakeup();
        }
        let mut st = self.lease.lock().unwrap_or_else(|e| e.into_inner());
        if st.lease.active()
            && let Ok(bus) = self.ends.bus()
        {
            bluetooth::end_lease(&bus, &self.ends.bluez, &mut st.lease);
        }
        drop(st);
        self.end_gamma();
    }
}

/// The watcher's one-shot emit (`watch`): every kind marked since, once.
fn arm_emit(
    context: &glib::MainContext,
    dirty: &Arc<Mutex<BTreeSet<&'static str>>>,
    weak: &Weak<Devices>,
) {
    let (dirty, weak) = (dirty.clone(), weak.clone());
    glib::timeout_source_new(EMIT_DELAY, None, glib::Priority::DEFAULT, move || {
        let kinds = std::mem::take(&mut *dirty.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(me) = weak.upgrade() {
            for what in kinds {
                me.changed(what);
            }
        }
        glib::ControlFlow::Break
    })
    .attach(Some(context));
}

#[cfg(test)]
mod tests;
