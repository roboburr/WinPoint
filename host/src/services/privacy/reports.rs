//! Ubuntu's error reports (privacy.reports.*): whoopsie-preferences on the system bus, the daemon GNOME Settings'
//! Ubuntu Diagnostics page talks to ("Send error reports to Canonical": Never / Automatic / Manual). It checks polkit
//! `com.ubuntu.whoopsiepreferences.change` itself (auth_admin_keep; Ubuntu's com.ubuntu.desktop.rules says YES to
//! local active sudo users), so the writes go through `call_interactive` (notes/claude/settings-parity.md §5.8).
use crate::services::system::common;
use glib::prelude::*;
use serde_json::{Value, json};

pub(super) const NAME: &str = "com.ubuntu.WhoopsiePreferences";
const PATH: &str = "/com/ubuntu/WhoopsiePreferences";
const IFACE: &str = "com.ubuntu.WhoopsiePreferences";
const TIMEOUT_MS: i32 = 5000;

/// The drop-down's choice for the daemon's two properties (cc-diagnostics-panel.c, Ubuntu).
pub(super) fn mode(report: bool, automatic: bool) -> &'static str {
    match (report, automatic) {
        (false, _) => "never",
        (true, true) => "auto",
        (true, false) => "manual",
    }
}

/// What SetReportCrashes and SetAutomaticallyReportCrashes get for a choice.
pub(super) fn flags(mode: &str) -> Option<(bool, bool)> {
    match mode {
        "never" => Some((false, false)),
        "auto" => Some((true, true)),
        "manual" => Some((true, false)),
        _ => None,
    }
}

/// The daemon's state: `{available, mode, reportCrashes, automatic}`; `available: false` when it is not there.
pub(super) fn get(bus: &gio::DBusConnection, name: &str) -> Value {
    let props = common::call(
        bus,
        name,
        PATH,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(IFACE,).to_variant()),
        TIMEOUT_MS,
    );
    let Ok(props) = props else {
        return json!({ "available": false });
    };
    let dict = common::dictionary(&props.child_value(0));
    let flag = |key: &str| dict.get(key).and_then(|v| v.get::<bool>());
    match (flag("ReportCrashes"), flag("AutomaticallyReportCrashes")) {
        (Some(report), automatic) => {
            let automatic = automatic.unwrap_or(false);
            json!({
                "available": true,
                "mode": mode(report, automatic),
                "reportCrashes": report,
                "automatic": automatic,
            })
        }
        _ => json!({ "available": false }),
    }
}

/// Write a choice as GNOME Settings does: SetReportCrashes, then SetAutomaticallyReportCrashes, both interactive.
pub(super) fn set(
    bus: &gio::DBusConnection,
    name: &str,
    report: bool,
    automatic: bool,
    lang: crate::i18n::Lang,
) -> Result<(), crate::files::ApiError> {
    common::call_interactive(
        bus,
        name,
        PATH,
        IFACE,
        "SetReportCrashes",
        Some(&(report,).to_variant()),
        lang,
    )?;
    common::call_interactive(
        bus,
        name,
        PATH,
        IFACE,
        "SetAutomaticallyReportCrashes",
        Some(&(automatic,).to_variant()),
        lang,
    )?;
    Ok(())
}
