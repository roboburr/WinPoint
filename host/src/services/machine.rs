//! `machine.*` (batch E): the computer's name, its date and time, its locales and languages and the About details,
//! as GNOME Settings' About, Date & Time and Region & Language panels handle them (notes/claude/settings-parity.md
//! §5.6). Native Settings' Info, Datum en tijd, Taal en regio, Regionale notatie and Beheerinstellingen voor taal call
//! these.
//!
//! * machine/info.rs: `machine.info` (hostname1, os-release, GNOME Shell's ShellVersion (amendment 37), systemd's
//!   Virtualization, switcheroo-control's GPUs, UDisks2's drive sizes).
//! * machine/hostname.rs: `machine.setName` (hostname1 SetPrettyHostname + SetStaticHostname, the static name derived
//!   as GNOME derives it).
//! * machine/time.rs and machine/zones.rs: `machine.time.*` (timedate1 only: there is no timesync1 here, amendment 30;
//!   "Nu synchroniseren" is SetNTP(false) then SetNTP(true); the server comes from chrony's own files) and the zone
//!   list with Windows' "(UTC+01:00) Amsterdam" labels.
//! * machine/locale.rs: `machine.locales` (the generated locales of `localedef --list-archive`, named through
//!   iso-codes as gnome-desktop names them; with `{preview}` GNOME's Formats preview), `machine.login.*` (locale1).
//! * machine/languages.rs: `machine.languages` and `machine.languages.install` (language-selector's data files and
//!   PackageKit, amendment 59; the Python language-selector itself never runs).
//! * Events: `machine.changed {what}` after every write here ("name", "time", "login", "languages") and when
//!   hostname1, timedate1 or locale1 report a change made elsewhere; the language install reports its progress as
//!   `machine.changed {what: "languages", code, percent}` and its end with `done: true`.
//!
//! Writes go to the daemons, which ask polkit themselves (§5.8, `call_interactive`, and the methods' own
//! `interactive` argument true, as GNOME Settings passes it). Under `ctx.dry_run` a write checks its arguments and
//! calls nothing.
pub(crate) mod hostname;
pub(crate) mod info;
pub(crate) mod languages;
pub(crate) mod locale;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod time;
pub(crate) mod zones;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The methods (name, dangerous), exactly as host/src/api-methods/accounts.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("machine.info", false),
    ("machine.languages", false),
    ("machine.languages.install", true),
    ("machine.locales", false),
    ("machine.login.get", false),
    ("machine.login.setLocale", true),
    ("machine.setName", true),
    ("machine.time.get", false),
    ("machine.time.setNtp", true),
    ("machine.time.setTime", true),
    ("machine.time.setTimezone", true),
    ("machine.time.sync", true),
    ("machine.time.zones", false),
];

/// (well-known name, object path, interface) of the daemons.
pub(crate) const HOSTNAME: (&str, &str, &str) = (
    "org.freedesktop.hostname1",
    "/org/freedesktop/hostname1",
    "org.freedesktop.hostname1",
);
pub(crate) const TIMEDATE: (&str, &str, &str) = (
    "org.freedesktop.timedate1",
    "/org/freedesktop/timedate1",
    "org.freedesktop.timedate1",
);
pub(crate) const LOCALE: (&str, &str, &str) = (
    "org.freedesktop.locale1",
    "/org/freedesktop/locale1",
    "org.freedesktop.locale1",
);
pub(crate) const PACKAGEKIT: (&str, &str, &str) = (
    "org.freedesktop.PackageKit",
    "/org/freedesktop/PackageKit",
    "org.freedesktop.PackageKit",
);
pub(crate) const SYSTEMD: (&str, &str, &str) = (
    "org.freedesktop.systemd1",
    "/org/freedesktop/systemd1",
    "org.freedesktop.systemd1.Manager",
);
pub(crate) const SHELL: (&str, &str, &str) =
    ("org.gnome.Shell", "/org/gnome/Shell", "org.gnome.Shell");
pub(crate) const SWITCHEROO: (&str, &str, &str) = (
    "net.hadess.SwitcherooControl",
    "/net/hadess/SwitcherooControl",
    "net.hadess.SwitcherooControl",
);
pub(crate) const UDISKS: (&str, &str) = ("org.freedesktop.UDisks2", "/org/freedesktop/UDisks2");

/// How long a read of a system daemon waits (timedate1, hostname1 and locale1 are started on demand).
const READ_MS: i32 = 5000;

/// One daemon: a bus connection and the name to call (the well-known name, or a test fake's unique name).
#[derive(Clone)]
pub(crate) struct Peer {
    pub bus: gio::DBusConnection,
    pub name: String,
}

impl Peer {
    fn on(bus: Result<gio::DBusConnection, String>, name: &str) -> Option<Peer> {
        Some(Peer {
            bus: bus.ok()?,
            name: name.into(),
        })
    }

    /// A call that needs no authorization (a read).
    pub(crate) fn call(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant, String> {
        common::call(&self.bus, &self.name, path, iface, method, args, READ_MS)
    }

    /// Every property of `iface` as JSON.
    pub(crate) fn props(&self, path: &str, iface: &str) -> Result<Value, String> {
        let v = self.call(
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(iface,).to_variant()),
        )?;
        Ok(common::variant_json(&v.child_value(0)))
    }

    /// One property as JSON.
    pub(crate) fn prop(&self, path: &str, iface: &str, name: &str) -> Result<Value, String> {
        let v = self.call(
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(iface, name).to_variant()),
        )?;
        Ok(common::variant_json(&v.child_value(0)))
    }

    /// A write the daemon checks with polkit (GNOME's password dialog where the policy asks for one).
    pub(crate) fn interactive(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        args: &glib::Variant,
        lang: Lang,
    ) -> Result<glib::Variant, ApiError> {
        common::call_interactive(&self.bus, &self.name, path, iface, method, Some(args), lang)
    }
}

/// The daemons and programs this service uses: None is the system's own (tests give fakes on their private bus).
#[derive(Clone, Default)]
pub(crate) struct Buses {
    pub hostname: Option<Peer>,
    pub timedate: Option<Peer>,
    pub locale: Option<Peer>,
    pub packagekit: Option<Peer>,
    pub systemd: Option<Peer>,
    pub shell: Option<Peer>,
    pub switcheroo: Option<Peer>,
    pub udisks: Option<Peer>,
    /// `localedef` (tests: a script); None is `tools::system_tool("localedef")`.
    pub localedef: Option<PathBuf>,
    /// The root of the system's data files (tests: a fixture tree); None is `/`.
    pub root: Option<PathBuf>,
}

impl Buses {
    fn system(slot: &Option<Peer>, name: &str) -> Option<Peer> {
        slot.clone().or_else(|| Peer::on(common::system(), name))
    }

    pub(crate) fn hostname(&self) -> Option<Peer> {
        Self::system(&self.hostname, HOSTNAME.0)
    }
    pub(crate) fn timedate(&self) -> Option<Peer> {
        Self::system(&self.timedate, TIMEDATE.0)
    }
    pub(crate) fn locale(&self) -> Option<Peer> {
        Self::system(&self.locale, LOCALE.0)
    }
    pub(crate) fn packagekit(&self) -> Option<Peer> {
        Self::system(&self.packagekit, PACKAGEKIT.0)
    }
    pub(crate) fn systemd(&self) -> Option<Peer> {
        Self::system(&self.systemd, SYSTEMD.0)
    }
    pub(crate) fn switcheroo(&self) -> Option<Peer> {
        Self::system(&self.switcheroo, SWITCHEROO.0)
    }
    pub(crate) fn udisks(&self) -> Option<Peer> {
        Self::system(&self.udisks, UDISKS.0)
    }
    pub(crate) fn shell(&self) -> Option<Peer> {
        self.shell
            .clone()
            .or_else(|| Peer::on(common::session(), SHELL.0))
    }

    /// A system file under the root (`/etc/chrony/chrony.conf`, `/usr/share/zoneinfo/zone1970.tab`, ...).
    pub(crate) fn file(&self, path: &str) -> PathBuf {
        match &self.root {
            Some(root) => root.join(path.trim_start_matches('/')),
            None => PathBuf::from(path),
        }
    }
}

pub(crate) fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)
}

pub(crate) fn str_arg<'a>(args: &'a Value, key: &str, lang: Lang) -> Result<&'a str, ApiError> {
    args[key].as_str().ok_or_else(|| bad(lang))
}

fn bool_arg(args: &Value, key: &str, lang: Lang) -> Result<bool, ApiError> {
    args[key].as_bool().ok_or_else(|| bad(lang))
}

/// A daemon or program that is not there: `unsupported <key>`.
pub(crate) fn unavailable(lang: Lang, key: &str) -> ApiError {
    ApiError::tr("unsupported", lang, key, NO_PARAMS)
}

/// The `{ok, dryRun}` answer of a write under `--dry-run` (its arguments were checked).
pub(crate) fn dry() -> ApiResult {
    Ok(json!({"ok": true, "dryRun": true}))
}

pub(crate) struct Machine {
    ctx: Arc<ServiceContext>,
    buses: Buses,
    install: Arc<Mutex<Option<String>>>,
    watcher: Mutex<Option<Watcher>>,
    seen: Arc<Mutex<time::Seen>>,
}

impl Machine {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Self::with(ctx, Buses::default())
    }

    pub(crate) fn with(ctx: Arc<ServiceContext>, buses: Buses) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            buses,
            install: Arc::new(Mutex::new(None)),
            watcher: Mutex::new(None),
            seen: Arc::new(Mutex::new(time::Seen::default())),
        }))
    }

    fn changed(&self, what: &str) {
        self.ctx.emit("machine.changed", json!({ "what": what }));
    }

    /// Follow hostname1, timedate1 and locale1 from the first call on (the watcher thread, below).
    fn watch(&self) {
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Watcher::start(
                self.ctx.clone(),
                vec![
                    (self.buses.hostname(), HOSTNAME.1, "name"),
                    (self.buses.timedate(), TIMEDATE.1, "time"),
                    (self.buses.locale(), LOCALE.1, "login"),
                ],
                self.buses.clone(),
                self.seen.clone(),
            );
        }
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        self.watch();
        Some(self.dispatch(method, args, self.ctx.lang()))
    }

    fn dispatch(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let (b, dry) = (&self.buses, self.ctx.dry_run);
        let written = |what: &str, result: ApiResult| {
            if result.as_ref().is_ok_and(|v| v["dryRun"] != true) {
                self.changed(what);
            }
            result
        };
        match method {
            "machine.info" => Ok(info::info(b)),
            "machine.setName" => written(
                "name",
                hostname::set_name(b, str_arg(args, "name", lang)?, lang, dry),
            ),
            "machine.time.get" => Ok(time::get(b, &self.seen, lang)),
            "machine.time.zones" => Ok(zones::list(b, time::current_zone(b).as_deref(), lang)),
            "machine.time.setNtp" => written(
                "time",
                time::set_ntp(b, bool_arg(args, "on", lang)?, lang, dry),
            ),
            "machine.time.setTime" => {
                let usec = args["usec"].as_i64().ok_or_else(|| bad(lang))?;
                written("time", time::set_time(b, usec, lang, dry))
            }
            "machine.time.setTimezone" => written(
                "time",
                time::set_timezone(b, str_arg(args, "zone", lang)?, lang, dry),
            ),
            "machine.time.sync" => written("time", time::sync(b, &self.seen, lang, dry)),
            "machine.locales" => locale::locales(b, args, lang),
            "machine.login.get" => Ok(locale::login_get(b)),
            "machine.login.setLocale" => written("login", locale::login_set(b, args, lang, dry)),
            "machine.languages" => Ok(languages::list(b, &self.install, lang)),
            "machine.languages.install" => languages::install(
                &self.ctx,
                b,
                &self.install,
                str_arg(args, "code", lang)?,
                lang,
            ),
            _ => Err(bad(lang)),
        }
    }

    pub(crate) fn shutdown(&self) {
        if let Some(w) = self
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            w.stop();
        }
    }
}

// ------------------------------------------------------------------------------------------------ the watcher

/// A thread of its own that follows PropertiesChanged of hostname1, timedate1 and locale1: a change made elsewhere
/// (gsd-datetime's automatic time zone, chrony's synchronisation, another Settings) is `machine.changed {what}`.
struct Watcher {
    context: glib::MainContext,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    fn start(
        ctx: Arc<ServiceContext>,
        peers: Vec<(Option<Peer>, &'static str, &'static str)>,
        buses: Buses,
        seen: Arc<Mutex<time::Seen>>,
    ) -> Option<Watcher> {
        if peers.iter().all(|(p, _, _)| p.is_none()) {
            return None;
        }
        let context = glib::MainContext::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_context, thread_stop) = (context.clone(), stop.clone());
        let (ready, subscribed) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("win11-machine".into())
            .spawn(move || {
                let _ = thread_context.with_thread_default(|| {
                    let mut subscriptions = Vec::new();
                    for (peer, path, what) in peers {
                        let Some(peer) = peer else { continue };
                        let pending = Arc::new(AtomicBool::new(false));
                        let (ctx2, buses2, seen2) = (ctx.clone(), buses.clone(), seen.clone());
                        let id = peer.bus.signal_subscribe(
                            Some(&peer.name),
                            Some("org.freedesktop.DBus.Properties"),
                            Some("PropertiesChanged"),
                            Some(path),
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, _, _, _, _| {
                                // a burst of property changes is one event; the read is off this thread: it
                                // notes when NTPSynchronized turns true ("Laatste geslaagde
                                // tijdsynchronisatie"), then the pages read again
                                let (ctx3, buses3, seen3) =
                                    (ctx2.clone(), buses2.clone(), seen2.clone());
                                crate::services::users::debounce(
                                    &pending,
                                    std::time::Duration::from_millis(250),
                                    move || {
                                        if what == "time" {
                                            time::get(&buses3, &seen3, Lang::En);
                                        }
                                        ctx3.emit("machine.changed", json!({ "what": what }));
                                    },
                                );
                            },
                        );
                        subscriptions.push((peer.bus.clone(), id));
                    }
                    let _ = ready.send(());
                    while !thread_stop.load(Ordering::Acquire) {
                        thread_context.iteration(true);
                    }
                    for (bus, id) in subscriptions {
                        bus.signal_unsubscribe(id);
                    }
                });
            })
            .ok()?;
        let _ = subscribed.recv_timeout(std::time::Duration::from_secs(5));
        Some(Watcher { context, stop })
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
    }
}
