//! systemd's Manager, as gnome-control-center's cc-systemd-service.c uses it: the user manager on the session bus
//! (gnome-remote-desktop.service of Desktop Sharing) and the system manager on the system bus (ssh.socket and
//! ssh.service of Secure Shell). A service is turned on with StartUnit + EnableUnitFiles and off with StopUnit +
//! DisableUnitFiles; its state is the unit's ActiveState and UnitFileState. Writes on the system manager go through
//! `common::call_interactive` (polkit `org.freedesktop.systemd1.manage-units` / `manage-unit-files`, auth_admin_keep:
//! GNOME Shell's password dialog; Secure Shell acquires GNOME's `org.gnome.controlcenter.remote-login-helper` first,
//! which implies both, so its switch asks once: ssh.rs), the user manager needs no polkit.
use crate::files::ApiError;
use crate::i18n::Lang;
use crate::services::system::common::{call_interactive, interactive_error, variant_json};
use glib::variant::ToVariant;
use serde_json::{Value, json};

pub(crate) const SYSTEMD: &str = "org.freedesktop.systemd1";
pub(crate) const PATH: &str = "/org/freedesktop/systemd1";
pub(crate) const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT: &str = "org.freedesktop.systemd1.Unit";
/// Reads answer within this time (a manager that hangs never blocks the page).
const READ_MS: i32 = 3000;
/// The user manager's writes (no password dialog).
const USER_WRITE_MS: i32 = 30_000;

/// A unit's state: whether systemd knows it, its unit file state ("enabled", "disabled", "static", ...) and its
/// ActiveState ("active", "inactive", "failed", "activating", ...).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct UnitState {
    pub exists: bool,
    pub file_state: String,
    pub active: String,
}

impl UnitState {
    /// Running (or starting): what the switch shows.
    pub(crate) fn running(&self) -> bool {
        matches!(
            self.active.as_str(),
            "active" | "activating" | "reloading" | "refreshing"
        )
    }

    /// Started at boot.
    pub(crate) fn enabled(&self) -> bool {
        matches!(
            self.file_state.as_str(),
            "enabled" | "enabled-runtime" | "static" | "alias" | "indirect" | "generated"
        )
    }

    pub(crate) fn json(&self) -> Value {
        json!({
            "exists": self.exists, "fileState": self.file_state, "active": self.active,
            "running": self.running(), "enabled": self.enabled(),
        })
    }
}

/// A unit name WinPoint asks systemd about (its own constants only; never a name from the page).
pub(crate) fn unit_ok(unit: &str) -> bool {
    unit.len() <= 64
        && (unit.ends_with(".service") || unit.ends_with(".socket"))
        && unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
}

fn read(
    bus: &gio::DBusConnection,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<glib::Variant, String> {
    bus.call_sync(
        Some(SYSTEMD),
        path,
        iface,
        method,
        args,
        None,
        gio::DBusCallFlags::NONE,
        READ_MS,
        gio::Cancellable::NONE,
    )
    .map_err(|e| e.to_string())
}

/// The unit's state: GetUnitFileState for the file (an error: systemd has no such unit), then LoadUnit and the
/// unit's ActiveState.
pub(crate) fn state(bus: &gio::DBusConnection, unit: &str) -> UnitState {
    if !unit_ok(unit) {
        return UnitState::default();
    }
    let file_state = match read(
        bus,
        PATH,
        MANAGER,
        "GetUnitFileState",
        Some(&(unit,).to_variant()),
    ) {
        Ok(v) => v
            .child_value(0)
            .str()
            .map(str::to_owned)
            .unwrap_or_default(),
        Err(_) => return UnitState::default(),
    };
    let active = read(bus, PATH, MANAGER, "LoadUnit", Some(&(unit,).to_variant()))
        .ok()
        .and_then(|v| v.child_value(0).str().map(str::to_owned))
        .and_then(|path| {
            read(
                bus,
                &path,
                "org.freedesktop.DBus.Properties",
                "Get",
                Some(&(UNIT, "ActiveState").to_variant()),
            )
            .ok()
        })
        .map(|v| variant_json(&v.child_value(0)))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "inactive".to_owned());
    UnitState {
        exists: true,
        file_state,
        active,
    }
}

/// One Manager call: interactive (polkit, the system manager) or not (the user manager).
fn write(
    bus: &gio::DBusConnection,
    method: &str,
    args: &glib::Variant,
    interactive: bool,
    lang: Lang,
) -> Result<glib::Variant, ApiError> {
    if interactive {
        return call_interactive(bus, SYSTEMD, PATH, MANAGER, method, Some(args), lang);
    }
    bus.call_sync(
        Some(SYSTEMD),
        PATH,
        MANAGER,
        method,
        Some(args),
        None,
        gio::DBusCallFlags::NONE,
        USER_WRITE_MS,
        gio::Cancellable::NONE,
    )
    .map_err(|e| interactive_error(&e.to_string(), lang))
}

/// StartUnit(unit, "replace").
pub(crate) fn start(
    bus: &gio::DBusConnection,
    unit: &str,
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    write(
        bus,
        "StartUnit",
        &(unit, "replace").to_variant(),
        interactive,
        lang,
    )
    .map(|_| ())
}

/// StopUnit(unit, "replace").
pub(crate) fn stop(
    bus: &gio::DBusConnection,
    unit: &str,
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    write(
        bus,
        "StopUnit",
        &(unit, "replace").to_variant(),
        interactive,
        lang,
    )
    .map(|_| ())
}

/// EnableUnitFiles(units, runtime false, force false).
pub(crate) fn enable_files(
    bus: &gio::DBusConnection,
    units: &[&str],
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    write(
        bus,
        "EnableUnitFiles",
        &(units.to_vec(), false, false).to_variant(),
        interactive,
        lang,
    )
    .map(|_| ())
}

/// DisableUnitFiles(units, runtime false).
pub(crate) fn disable_files(
    bus: &gio::DBusConnection,
    units: &[&str],
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    write(
        bus,
        "DisableUnitFiles",
        &(units.to_vec(), false).to_variant(),
        interactive,
        lang,
    )
    .map(|_| ())
}

/// Turn a unit on as GNOME does: StartUnit(unit, "replace"), then EnableUnitFiles([unit], runtime false, force false).
pub(crate) fn enable(
    bus: &gio::DBusConnection,
    unit: &str,
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    start(bus, unit, interactive, lang)?;
    enable_files(bus, &[unit], interactive, lang)
}

/// Turn a unit off as GNOME does: StopUnit(unit, "replace"), then DisableUnitFiles([unit], runtime false).
pub(crate) fn disable(
    bus: &gio::DBusConnection,
    unit: &str,
    interactive: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    stop(bus, unit, interactive, lang)?;
    disable_files(bus, &[unit], interactive, lang)
}
