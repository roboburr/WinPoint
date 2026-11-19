//! `sharing.*` (batch C): Remote Desktop (gnome-remote-desktop's user daemon and its system daemon for remote
//! login), Secure Shell (the OpenSSH server's units, its install) and Media Sharing (rygel through
//! gnome-settings-daemon's Sharing plugin), for Instellingen › Systeem › Extern bureaublad, › Geavanceerd and
//! › Netwerk en internet › Geavanceerde instellingen voor delen (notes/claude/settings-parity.md §5.4).
//!
//! As GNOME Settings: the Desktop Sharing switch writes `org.gnome.desktop.remote-desktop.rdp` (allow/network.rs:
//! `enable` and `view-only` are Write rows, the certificate keys belong to `sharing.rdp.set`) and starts or stops the
//! user unit; the credentials go to the login keyring through grdctl (`tools::system_tool`, the password on its
//! standard input); the TLS certificate is made with GnuTLS (amendment 34, `sharing/tls.rs`); Remote Login, the SSH
//! units and the install are system writes through daemons that check polkit (`common::call_interactive`, §5.8).
//! The dangerous methods check their arguments under `ctx.dry_run` and touch nothing; every write announces
//! `sharing.changed {what}` (rdp, system, ssh, media), and a watcher announces gsd Sharing's and the RDP daemon's own
//! changes while a page listens. Methods that carry a password are `"secret": true` in
//! host/src/api-methods/network.json: nothing here logs their arguments, and their errors never hold one.
mod media;
mod rdp;
mod ssh;
mod systemd;
#[cfg(test)]
mod tests;
mod tls;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The methods (name, dangerous), exactly as host/src/api-methods/network.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("sharing.rdp.get", false),
    ("sharing.rdp.set", true),
    ("sharing.rdp.credentials.get", false),
    ("sharing.rdp.credentials.set", true),
    ("sharing.rdp.fingerprint", false),
    ("sharing.rdp.system.get", false),
    ("sharing.rdp.system.set", true),
    ("sharing.rdp.system.credentials.set", true),
    ("sharing.ssh.get", false),
    ("sharing.ssh.set", true),
    ("sharing.ssh.install", true),
    ("sharing.media.get", false),
    ("sharing.media.set", true),
    ("sharing.media.folders.set", true),
    ("sharing.media.networks.remove", true),
];

/// Where the service finds its tools and files (tests give their own).
pub(crate) struct Paths {
    /// grdctl through `tools::system_tool` (None: refused or not installed).
    pub grdctl: Option<PathBuf>,
    /// GNOME's certificate directory, `~/.local/share/gnome-remote-desktop/certificates`.
    pub cert_dir: PathBuf,
}

impl Paths {
    fn system() -> Paths {
        Paths {
            grdctl: crate::tools::system_tool("grdctl"),
            cert_dir: glib::user_data_dir()
                .join("gnome-remote-desktop")
                .join("certificates"),
        }
    }
}

pub(crate) struct Sharing {
    ctx: Arc<ServiceContext>,
    paths: Paths,
    stopped: Arc<AtomicBool>,
    installing: AtomicBool,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "sharing.badArgs", NO_PARAMS)
}

fn bool_arg(v: &Value, lang: Lang) -> Result<bool, ApiError> {
    v.as_bool().ok_or_else(|| bad(lang))
}

fn session() -> Option<gio::DBusConnection> {
    crate::services::system::common::session().ok()
}

fn system() -> Option<gio::DBusConnection> {
    crate::services::system::common::system().ok()
}

fn no_bus(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "sharing.unavailable", NO_PARAMS)
}

/// The checked credentials of a set call.
fn credentials_arg(args: &Value, lang: Lang) -> Result<(String, String), ApiError> {
    let wrong = || ApiError::tr("bad_args", lang, "sharing.badCredentials", NO_PARAMS);
    let username = args["username"]
        .as_str()
        .filter(|u| rdp::username_ok(u))
        .ok_or_else(wrong)?;
    let password = args["password"]
        .as_str()
        .filter(|p| rdp::password_ok(p))
        .ok_or_else(wrong)?;
    Ok((username.to_owned(), password.to_owned()))
}

/// A connection uuid (NetworkManager's, as gsd Sharing keeps them).
fn uuid_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|s| super::net::uuid_ok(s))
        .map(str::to_owned)
        .ok_or_else(|| bad(lang))
}

impl Sharing {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Self::with_paths(ctx, Paths::system())
    }

    pub(crate) fn with_paths(ctx: Arc<ServiceContext>, paths: Paths) -> Result<Arc<Self>, String> {
        let me = Arc::new(Self {
            ctx,
            paths,
            stopped: Arc::new(AtomicBool::new(false)),
            installing: AtomicBool::new(false),
        });
        let (weak, stopped) = (Arc::downgrade(&me), me.stopped.clone());
        std::thread::Builder::new()
            .name("sharing-watch".into())
            .spawn(move || watch(weak, stopped))
            .map_err(|e| e.to_string())?;
        Ok(me)
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn changed(&self, what: &str) {
        self.ctx.emit("sharing.changed", json!({ "what": what }));
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        Some(self.dispatch(method, args, self.ctx.lang()))
    }

    fn grdctl(&self) -> Option<&Path> {
        self.paths.grdctl.as_deref()
    }

    fn dispatch(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let dry = self.ctx.dry_run;
        match method {
            "sharing.rdp.get" => Ok(rdp::get(session().as_ref(), self.grdctl(), lang)),
            "sharing.rdp.set" => {
                let on = bool_arg(&args["on"], lang)?;
                if dry {
                    return Ok(json!({ "on": on }));
                }
                let bus = session().ok_or_else(|| no_bus(lang))?;
                let r = rdp::set(&bus, on, &self.paths.cert_dir, self.grdctl(), lang);
                self.changed("rdp");
                let outcome = r?;
                Ok(json!({ "on": on, "credentials": outcome.credentials }))
            }
            "sharing.rdp.credentials.get" => {
                if args["system"] == true {
                    let bus = system().ok_or_else(|| no_bus(lang))?;
                    let (username, password) = rdp::system_credentials(&bus, lang)?;
                    return Ok(
                        json!({ "username": username, "password": password, "system": true }),
                    );
                }
                let (username, password) = rdp::credentials(self.grdctl(), lang)?;
                Ok(json!({ "username": username, "password": password }))
            }
            "sharing.rdp.credentials.set" => {
                let (username, password) = credentials_arg(args, lang)?;
                if dry {
                    return Ok(json!({ "username": username }));
                }
                rdp::set_credentials(self.grdctl(), &username, &password, lang)?;
                self.changed("rdp");
                Ok(json!({ "username": username }))
            }
            "sharing.rdp.fingerprint" => {
                let state = rdp::get(None, None, lang);
                if state["available"] != true {
                    return Err(ApiError::tr(
                        "unsupported",
                        lang,
                        "sharing.noRemoteDesktop",
                        NO_PARAMS,
                    ));
                }
                Ok(json!({ "fingerprint": state["fingerprint"] }))
            }
            "sharing.rdp.system.get" => Ok(rdp::system_get(system().as_ref())),
            "sharing.rdp.system.set" => {
                let on = bool_arg(&args["on"], lang)?;
                if dry {
                    return Ok(json!({ "on": on }));
                }
                let bus = system().ok_or_else(|| no_bus(lang))?;
                let r = rdp::system_set(&bus, on, lang);
                self.changed("system");
                r
            }
            "sharing.rdp.system.credentials.set" => {
                let (username, password) = credentials_arg(args, lang)?;
                if dry {
                    return Ok(json!({ "username": username }));
                }
                let bus = system().ok_or_else(|| no_bus(lang))?;
                rdp::system_set_credentials(&bus, &username, &password, lang)?;
                self.changed("system");
                Ok(json!({ "username": username }))
            }
            "sharing.ssh.get" => Ok(ssh::get(
                system().as_ref(),
                self.installing.load(Ordering::Acquire),
            )),
            "sharing.ssh.set" => {
                let on = bool_arg(&args["on"], lang)?;
                if dry {
                    return Ok(json!({ "on": on }));
                }
                let bus = system().ok_or_else(|| no_bus(lang))?;
                let r = ssh::set(&bus, on, lang);
                self.changed("ssh");
                r.map(|()| json!({ "on": on }))
            }
            "sharing.ssh.install" => self.install(lang, dry),
            "sharing.media.get" => Ok(media::get(session().as_ref())),
            "sharing.media.set" => {
                let on = bool_arg(&args["on"], lang)?;
                if dry {
                    return Ok(json!({ "on": on }));
                }
                let bus = session().ok_or_else(|| no_bus(lang))?;
                let r = media::set(&bus, on, lang);
                self.changed("media");
                r.map(|()| json!({ "on": on }))
            }
            "sharing.media.folders.set" => {
                let folders = media::folders_arg(&args["folders"], lang)?;
                if !dry {
                    media::set_folders(&folders, lang)?;
                    self.changed("media");
                }
                Ok(json!({ "folders": folders }))
            }
            "sharing.media.networks.remove" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                if !dry {
                    let bus = session().ok_or_else(|| no_bus(lang))?;
                    media::remove_network(&bus, &uuid, lang)?;
                    self.changed("media");
                }
                Ok(json!({ "uuid": uuid }))
            }
            _ => Err(bad(lang)),
        }
    }

    /// sharing.ssh.install: one at a time; `sharing.changed {what: "sshInstall", percentage}` while it runs.
    fn install(&self, lang: Lang, dry: bool) -> ApiResult {
        if dry {
            return Ok(json!({ "package": ssh::PACKAGE }));
        }
        if self.installing.swap(true, Ordering::AcqRel) {
            return Err(ApiError::tr(
                "busy",
                lang,
                "sharing.installRunning",
                NO_PARAMS,
            ));
        }
        let r = system().ok_or_else(|| no_bus(lang)).and_then(|bus| {
            let ctx = self.ctx.clone();
            ssh::install(
                &bus,
                &move |p| {
                    ctx.emit(
                        "sharing.changed",
                        json!({ "what": "sshInstall", "percentage": p }),
                    )
                },
                lang,
            )
        });
        self.installing.store(false, Ordering::Release);
        self.changed("ssh");
        r
    }
}

/// The watcher: gsd Sharing's property changes (the current network, its status) become `sharing.changed {what:
/// "media"}`, the RDP daemon's (its port, enabled) `{what: "rdp"}`, the Remote Login configuration daemon's on the
/// system bus (enabled, port, certificate: another program such as GNOME Settings or `grdctl --system` changed it)
/// `{what: "system"}`, at most every 400 ms, while someone listens.
fn watch(weak: Weak<Sharing>, stopped: Arc<AtomicBool>) {
    type Dirty = Arc<std::sync::Mutex<std::collections::BTreeSet<&'static str>>>;
    let context = glib::MainContext::new();
    let _ = context.with_thread_default(|| {
        let Some(bus) = session() else { return };
        let dirty: Dirty = Arc::default();
        let subscribe = |bus: &gio::DBusConnection, sender: &str, what: &'static str| {
            let sink = dirty.clone();
            bus.signal_subscribe(
                Some(sender),
                None,
                None,
                None,
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, _| {
                    sink.lock().unwrap_or_else(|e| e.into_inner()).insert(what);
                },
            )
        };
        let subs: Vec<gio::SignalSubscriptionId> = [
            (media::GSD, "media"),
            ("org.gnome.RemoteDesktop.User", "rdp"),
        ]
        .into_iter()
        .map(|(sender, what)| subscribe(&bus, sender, what))
        .collect();
        let system_bus = system();
        let system_sub = system_bus
            .as_ref()
            .map(|b| subscribe(b, rdp::CONFIG_NAME, "system"));
        let mut last = Instant::now();
        while !stopped.load(Ordering::Acquire) {
            while context.pending() {
                context.iteration(false);
            }
            if last.elapsed() >= Duration::from_millis(400) {
                last = Instant::now();
                let Some(me) = weak.upgrade() else { break };
                let kinds = std::mem::take(&mut *dirty.lock().unwrap_or_else(|e| e.into_inner()));
                if me.ctx.subscriber_count() > 0 {
                    for kind in kinds {
                        me.changed(kind);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for sub in subs {
            bus.signal_unsubscribe(sub);
        }
        if let (Some(b), Some(sub)) = (system_bus, system_sub) {
            b.signal_unsubscribe(sub);
        }
    });
}
