//! Secure Shell: GNOME Settings' System › Secure Shell (the OpenSSH server's units through systemd's system manager)
//! and Windows' "Een optionele functie toevoegen" for a computer without openssh-server (PackageKit InstallPackages,
//! polkit `org.freedesktop.packagekit.package-install`).
//!
//! The password dialog as GNOME Settings 50 has it: the switch first acquires polkit's
//! `org.gnome.controlcenter.remote-login-helper` (gnome-control-center's own action, auth_admin_keep, "Authentication
//! is required to enable or disable remote login"; its policy implies `org.freedesktop.systemd1.manage-units` and
//! `manage-unit-files`), then calls systemd, which polkit then lets through without asking again: one dialog per
//! switch, never a second one that could be cancelled halfway. Without that action (no gnome-control-center, or no
//! polkit on the bus) each systemd call asks on its own, as before.
//!
//! Amendment 35: Ubuntu's openssh-server is socket-activated. On turns on `ssh.socket` when systemd has it (else
//! `ssh.service`); off disables both unit files first and then stops both, so a refusal never leaves a unit enabled
//! for the next boot. Ubuntu's openssh-server postinst enables and starts ssh.socket itself; Windows' OpenSSH Server
//! optional feature arrives stopped, so the install turns the server off again (under the same authorization) and
//! the user turns it on with the switch.
use super::systemd::{self, UnitState};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{
    INTERACTIVE_TIMEOUT_MS, interactive_error, remote_error_name,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const SOCKET: &str = "ssh.socket";
pub(crate) const SERVICE: &str = "ssh.service";
pub(crate) const PACKAGE: &str = "openssh-server";
pub(crate) const SSHD: &str = "/usr/sbin/sshd";
const PK: &str = "org.freedesktop.PackageKit";
const PK_PATH: &str = "/org/freedesktop/PackageKit";
const PK_TX: &str = "org.freedesktop.PackageKit.Transaction";
/// PkFilterEnum bits: not installed (3) and the native architecture (18).
pub(crate) const FILTER_NOT_INSTALLED_ARCH: u64 = (1 << 3) | (1 << 18);
/// PkTransactionFlagEnum bit: only trusted packages (1).
pub(crate) const FLAG_ONLY_TRUSTED: u64 = 1 << 1;
/// PkExitEnum success.
const EXIT_SUCCESS: u32 = 1;
/// An install waits this long at most (a slow mirror).
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The SSH command GNOME shows: `ssh <user>@<host>`.
pub(crate) fn command(user: &str, host: &str) -> String {
    format!("ssh {user}@{host}")
}

/// The switch's state from both units: on while either runs.
pub(crate) fn summary(
    socket: &UnitState,
    service: &UnitState,
    sshd: bool,
    installing: bool,
) -> Value {
    json!({
        "installed": sshd || socket.exists || service.exists,
        "on": socket.running() || service.running(),
        "enabled": socket.enabled() || service.enabled(),
        "socket": socket.json(),
        "service": service.json(),
        "installing": installing,
    })
}

/// sharing.ssh.get.
pub(crate) fn get(bus: Option<&gio::DBusConnection>, installing: bool) -> Value {
    let (socket, service) = match bus {
        Some(b) => (systemd::state(b, SOCKET), systemd::state(b, SERVICE)),
        None => (UnitState::default(), UnitState::default()),
    };
    let mut v = summary(
        &socket,
        &service,
        std::path::Path::new(SSHD).exists(),
        installing,
    );
    v["command"] = json!(command(&super::rdp::user_name(), &super::rdp::hostname()));
    v
}

/// GNOME Settings' polkit action for the Secure Shell switch (org.gnome.controlcenter.remote-login-helper.policy).
pub(crate) const POLKIT_ACTION: &str = "org.gnome.controlcenter.remote-login-helper";
const POLKIT: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";
/// PolkitCheckAuthorizationFlags: AllowUserInteraction.
const ALLOW_USER_INTERACTION: u32 = 1;

/// CheckAuthorization's arguments as GNOME's GPermission sends them, for WinPoint's own bus name: subject
/// `("system-bus-name", {"name": <unique name>})`, the action, no details, AllowUserInteraction, no cancellation id.
pub(crate) fn check_args(unique_name: &str) -> glib::Variant {
    let subject: HashMap<String, glib::Variant> =
        HashMap::from([("name".to_owned(), unique_name.to_variant())]);
    let details: HashMap<String, String> = HashMap::new();
    (
        ("system-bus-name", subject),
        POLKIT_ACTION,
        details,
        ALLOW_USER_INTERACTION,
        "",
    )
        .to_variant()
}

/// What polkit's answer means: Ok when authorized; polkit.dismissed (the dialog closed) is `core.authCancelled`, any
/// other refusal `core.notAuthorized`.
pub(crate) fn authorization_outcome(
    authorized: bool,
    details: &HashMap<String, String>,
    lang: Lang,
) -> Result<(), ApiError> {
    if authorized {
        Ok(())
    } else if details.contains_key("polkit.dismissed") {
        Err(ApiError::tr(
            "denied",
            lang,
            "core.authCancelled",
            NO_PARAMS,
        ))
    } else {
        Err(ApiError::tr(
            "denied",
            lang,
            "core.notAuthorized",
            NO_PARAMS,
        ))
    }
}

/// A CheckAuthorization error after which the systemd calls ask themselves: polkit does not know GNOME's action (no
/// gnome-control-center installed: "Action ... is not registered") or polkit is not on the bus.
pub(crate) fn no_such_action(error: &str) -> bool {
    let name = remote_error_name(error).unwrap_or("");
    (name == "org.freedesktop.PolicyKit1.Error.Failed" && error.contains("not registered"))
        || matches!(
            name,
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
}

/// Acquire GNOME's remote-login action for this process (one password dialog, kept by polkit for a few minutes;
/// the implied systemd actions then pass without asking). Ok(true): authorized; Ok(false): the action is not there,
/// the systemd calls ask on their own; Err: refused or cancelled.
pub(crate) fn authorize(bus: &gio::DBusConnection, lang: Lang) -> Result<bool, ApiError> {
    let Some(unique) = bus.unique_name() else {
        return Ok(false);
    };
    let reply = bus.call_sync(
        Some(POLKIT),
        POLKIT_PATH,
        POLKIT_AUTHORITY,
        "CheckAuthorization",
        Some(&check_args(unique.as_str())),
        Some(glib::VariantTy::new("((bba{ss}))").expect("CheckAuthorization's reply type")),
        gio::DBusCallFlags::NONE,
        INTERACTIVE_TIMEOUT_MS,
        gio::Cancellable::NONE,
    );
    match reply {
        Ok(r) => {
            let result = r.child_value(0);
            let authorized = result.child_value(0).get::<bool>().unwrap_or(false);
            let details: HashMap<String, String> = result.child_value(2).get().unwrap_or_default();
            authorization_outcome(authorized, &details, lang).map(|()| true)
        }
        Err(e) if no_such_action(&e.to_string()) => Ok(false),
        Err(e) => Err(interactive_error(&e.to_string(), lang)),
    }
}

/// The units' writes after the authorization: on = StartUnit + EnableUnitFiles of the socket (else the service);
/// off = DisableUnitFiles of every unit there, then StopUnit of each.
fn switch(
    bus: &gio::DBusConnection,
    on: bool,
    socket: &UnitState,
    service: &UnitState,
    lang: Lang,
) -> Result<(), ApiError> {
    if on {
        let unit = if socket.exists { SOCKET } else { SERVICE };
        return systemd::enable(bus, unit, true, lang);
    }
    let units: Vec<&str> = [(SOCKET, socket), (SERVICE, service)]
        .into_iter()
        .filter(|(_, s)| s.exists)
        .map(|(u, _)| u)
        .collect();
    systemd::disable_files(bus, &units, true, lang)?;
    for unit in units {
        systemd::stop(bus, unit, true, lang)?;
    }
    Ok(())
}

/// sharing.ssh.set {on}.
pub(crate) fn set(bus: &gio::DBusConnection, on: bool, lang: Lang) -> Result<(), ApiError> {
    let (socket, service) = (systemd::state(bus, SOCKET), systemd::state(bus, SERVICE));
    if !socket.exists && !service.exists {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "sharing.noSsh",
            NO_PARAMS,
        ));
    }
    authorize(bus, lang)?;
    switch(bus, on, &socket, &service, lang)
}

/// The package id of openssh-server among Resolve's Package signals: `name;version;arch;data`.
pub(crate) fn pick_package(ids: &[String]) -> Option<String> {
    ids.iter()
        .find(|id| id.split(';').next() == Some(PACKAGE) && id.split(';').count() == 4)
        .cloned()
}

/// What a transaction reported.
#[derive(Default)]
pub(crate) struct Outcome {
    pub packages: Vec<String>,
    pub exit: Option<u32>,
    pub error: Option<(u32, String)>,
}

/// One PackageKit transaction on `bus`: CreateTransaction, SetHints, then `method(args)`; its Package, ErrorCode,
/// Finished signals and Percentage changes (`progress`) until it finishes or `timeout` passes. The transaction's
/// method is interactive (PackageKit asks polkit, GNOME Shell shows the password dialog).
pub(crate) fn transaction(
    bus: &gio::DBusConnection,
    method: &str,
    args: &glib::Variant,
    hints: &[String],
    timeout: Duration,
    progress: &dyn Fn(u32),
    lang: Lang,
) -> Result<Outcome, ApiError> {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let tid = crate::services::system::common::call(
                bus,
                PK,
                PK_PATH,
                PK,
                "CreateTransaction",
                None,
                10_000,
            )
            .map_err(|e| failed(&e, lang))?
            .child_value(0)
            .str()
            .map(str::to_owned)
            .ok_or_else(|| failed("no transaction", lang))?;
            let outcome: Arc<Mutex<Outcome>> = Arc::default();
            let percent: Arc<Mutex<Option<u32>>> = Arc::default();
            let (sink, psink) = (outcome.clone(), percent.clone());
            let sub = bus.signal_subscribe(
                Some(PK),
                None,
                None,
                Some(&tid),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, iface, member, params| {
                    let mut o = sink.lock().unwrap_or_else(|e| e.into_inner());
                    match (iface, member) {
                        (PK_TX, "Package") => {
                            if let Some(id) = params
                                .try_child_value(1)
                                .and_then(|v| v.str().map(str::to_owned))
                            {
                                o.packages.push(id);
                            }
                        }
                        (PK_TX, "ErrorCode") => {
                            let code = params
                                .try_child_value(0)
                                .and_then(|v| v.get::<u32>())
                                .unwrap_or(0);
                            let text = params
                                .try_child_value(1)
                                .and_then(|v| v.str().map(str::to_owned))
                                .unwrap_or_default();
                            o.error = Some((code, text));
                        }
                        (PK_TX, "Finished") => {
                            o.exit = Some(
                                params
                                    .try_child_value(0)
                                    .and_then(|v| v.get::<u32>())
                                    .unwrap_or(0),
                            );
                        }
                        ("org.freedesktop.DBus.Properties", "PropertiesChanged") => {
                            let changed = params.try_child_value(1);
                            if let Some(p) = changed
                                .map(|d| crate::services::system::common::variant_json(&d))
                                .and_then(|d| d["Percentage"].as_u64())
                                .and_then(|p| u32::try_from(p).ok())
                            {
                                *psink.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
                            }
                        }
                        _ => {}
                    }
                },
            );
            let run = (|| {
                crate::services::system::common::call(
                    bus,
                    PK,
                    &tid,
                    PK_TX,
                    "SetHints",
                    Some(&(hints.to_vec(),).to_variant()),
                    10_000,
                )
                .map_err(|e| failed(&e, lang))?;
                crate::services::system::common::call_interactive(
                    bus,
                    PK,
                    &tid,
                    PK_TX,
                    method,
                    Some(args),
                    lang,
                )?;
                let end = Instant::now() + timeout;
                let mut shown = None;
                loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    if let Some(p) = *percent.lock().unwrap_or_else(|e| e.into_inner())
                        && p <= 100
                        && shown != Some(p)
                    {
                        shown = Some(p);
                        progress(p);
                    }
                    if outcome
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .exit
                        .is_some()
                    {
                        return Ok(());
                    }
                    if Instant::now() >= end {
                        return Err(failed("timed out", lang));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })();
            bus.signal_unsubscribe(sub);
            run?;
            let o = std::mem::take(&mut *outcome.lock().unwrap_or_else(|e| e.into_inner()));
            Ok(o)
        })
        .map_err(|_| failed("main context", lang))?
}

fn failed(error: &str, lang: Lang) -> ApiError {
    ApiError::tr(
        "failed",
        lang,
        "sharing.installFailed",
        &json!({ "error": error.chars().take(300).collect::<String>() }),
    )
}

/// sharing.ssh.install: Resolve openssh-server (not installed, this architecture), then InstallPackages (only
/// trusted packages), with GNOME Software's hints (interactive, the UI language).
pub(crate) fn install(bus: &gio::DBusConnection, progress: &dyn Fn(u32), lang: Lang) -> ApiResult {
    let locale = match lang {
        Lang::Nl => "nl_NL.UTF-8",
        _ => "en_GB.UTF-8",
    };
    let hints = vec!["interactive=true".to_owned(), format!("locale={locale}")];
    let resolved = transaction(
        bus,
        "Resolve",
        &(FILTER_NOT_INSTALLED_ARCH, vec![PACKAGE]).to_variant(),
        &hints,
        Duration::from_secs(120),
        &|_| {},
        lang,
    )?;
    let Some(id) = pick_package(&resolved.packages) else {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "sharing.noPackage",
            NO_PARAMS,
        ));
    };
    let done = transaction(
        bus,
        "InstallPackages",
        &(FLAG_ONLY_TRUSTED, vec![id.as_str()]).to_variant(),
        &hints,
        INSTALL_TIMEOUT,
        progress,
        lang,
    )?;
    match (done.exit, &done.error) {
        (Some(EXIT_SUCCESS), _) => {}
        (_, Some((_, text))) => return Err(failed(text, lang)),
        (exit, None) => return Err(failed(&format!("exit {}", exit.unwrap_or(0)), lang)),
    }
    // Ubuntu's postinst enabled and started ssh.socket; Windows' optional feature arrives stopped: off again, and
    // when that is refused (the dialog cancelled), the answer says the server is on, so the page shows it
    let on = match after_install(bus, lang) {
        Ok(state) => state,
        Err(_) => true,
    };
    Ok(json!({ "installed": true, "package": id, "on": on }))
}

/// After the install: the server off when the package left a unit running or enabled; whether it is still on.
fn after_install(bus: &gio::DBusConnection, lang: Lang) -> Result<bool, ApiError> {
    let (socket, service) = (systemd::state(bus, SOCKET), systemd::state(bus, SERVICE));
    if !left_on(&socket, &service) {
        return Ok(false);
    }
    authorize(bus, lang)?;
    switch(bus, false, &socket, &service, lang)?;
    Ok(false)
}

/// Whether a freshly installed server would listen now or after the next boot.
pub(crate) fn left_on(socket: &UnitState, service: &UnitState) -> bool {
    [socket, service].iter().any(|u| {
        u.exists && (u.running() || matches!(u.file_state.as_str(), "enabled" | "enabled-runtime"))
    })
}
