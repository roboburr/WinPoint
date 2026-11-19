//! `input.screenReader.preferences` (batch A, notes/claude/settings-parity.md amendment 36): Verteller's "Instellingen
//! voor Verteller openen", GNOME Settings 50's Screen Reader "Configure".
//!
//! GNOME Settings (cc-ua-seeing-page.c `configure_screen_reader_activated_cb`) calls `ShowPreferences` of the running
//! Orca on the session bus (`org.gnome.Orca.Service`, `/org/gnome/Orca/Service`, interface `org.gnome.Orca.Service`,
//! Orca 50's `orca/dbus_service.py`), and its Configure row is sensitive only while the screen reader is on. Orca
//! then shows its own preferences window (Python and GTK 3, in Orca's process), so nothing of Orca runs under a
//! WinPoint process and launchBridge.js is not needed for it.
//!
//! One plain call with `NO_AUTO_START` (Orca's name is not D-Bus activatable; GNOME Shell starts Orca when
//! `screen-reader-enabled` turns on) and without the interactive-authorization flag (no polkit is involved). Answers:
//! `{shown: true}`; no Orca on the bus `unsupported input.orcaNotRunning`; Orca answering false (no script to show the
//! preferences of) or any other error `failed input.orcaFailed`.
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::remote_error_name;
use serde_json::json;

pub(crate) const BUS_NAME: &str = "org.gnome.Orca.Service";
pub(crate) const OBJECT_PATH: &str = "/org/gnome/Orca/Service";
pub(crate) const INTERFACE: &str = "org.gnome.Orca.Service";
/// ShowPreferences builds Orca's window before it answers.
const TIMEOUT_MS: i32 = 10_000;

/// Whether a D-Bus error means that nobody owns the name (Orca is not running).
fn not_running(error: &glib::Error) -> bool {
    matches!(
        remote_error_name(&error.to_string()),
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        )
    )
}

/// `ShowPreferences` on `destination` (Orca's well-known name; a fake's unique name in tests).
pub(crate) fn show_preferences_on(
    bus: &gio::DBusConnection,
    destination: &str,
    lang: Lang,
) -> ApiResult {
    let reply_type = glib::VariantTy::new("(b)").ok();
    let reply = bus.call_sync(
        Some(destination),
        OBJECT_PATH,
        INTERFACE,
        "ShowPreferences",
        None,
        reply_type,
        gio::DBusCallFlags::NO_AUTO_START,
        TIMEOUT_MS,
        gio::Cancellable::NONE,
    );
    match reply {
        Ok(v) if v.get::<(bool,)>() == Some((true,)) => Ok(json!({ "shown": true })),
        Ok(_) => Err(ApiError::tr("failed", lang, "input.orcaFailed", NO_PARAMS)),
        Err(e) if not_running(&e) => Err(ApiError::tr(
            "unsupported",
            lang,
            "input.orcaNotRunning",
            NO_PARAMS,
        )),
        Err(_) => Err(ApiError::tr("failed", lang, "input.orcaFailed", NO_PARAMS)),
    }
}

/// `input.screenReader.preferences {}`: Orca's preferences on the session bus.
pub(crate) fn show_preferences(lang: Lang) -> ApiResult {
    match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(bus) => show_preferences_on(&bus, BUS_NAME, lang),
        Err(_) => Err(ApiError::tr(
            "unsupported",
            lang,
            "input.orcaNotRunning",
            NO_PARAMS,
        )),
    }
}
