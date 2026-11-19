//! Every admin write through cups-pk-helper (org.opensuse.CupsPkHelper.Mechanism on the system bus), as GNOME's
//! printers panel does (pp-utils.c, pp-new-printer.c, pp-printer.c, pp-job.c): the mechanism checks polkit itself
//! (`org.opensuse.cupspkhelper.mechanism.*`: YES for local sudo and lpadmin users through Ubuntu's
//! com.ubuntu.desktop.rules, otherwise auth_admin_keep; job-edit YES; settings-parity.md §5.8), so each call goes
//! with interactive authorization (as `call_interactive`) and GNOME Shell's password dialog. Each method answers `(s error)`: an empty string is
//! success, any other text is CUPS's refusal.
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use crate::services::system::common::{
    INTERACTIVE_TIMEOUT_MS, first, interactive_error, remote_error_name, string,
};
use serde_json::{Value, json};

pub(crate) const PATH: &str = "/";
pub(crate) const IFACE: &str = "org.opensuse.CupsPkHelper.Mechanism";

/// A failed call: cups-pk-helper's NotPrivileged is polkit's no, a missing mechanism is "not available", the rest
/// as `call_interactive` maps it (core.authCancelled, core.notAuthorized, core.callFailed).
pub(crate) fn mechanism_error(text: &str, lang: Lang) -> ApiError {
    let name = remote_error_name(text).unwrap_or("").to_ascii_lowercase();
    if name.ends_with(".notprivileged") {
        return ApiError::tr("denied", lang, "core.notAuthorized", crate::i18n::NO_PARAMS);
    }
    if name.ends_with(".serviceunknown") || name.ends_with(".namehasnoowner") {
        return ApiError::tr(
            "unsupported",
            lang,
            "printers.noHelper",
            crate::i18n::NO_PARAMS,
        );
    }
    interactive_error(text, lang)
}

/// Call a mechanism method with interactive authorization (as `call_interactive`: up to 10 minutes for the password
/// dialog); its `(s error)` (or `(s error, a{ss})` for DevicesGet), an empty error string as success.
pub(crate) fn call(
    bus: &gio::DBusConnection,
    name: &str,
    method: &str,
    args: glib::Variant,
    lang: Lang,
) -> Result<glib::Variant, ApiError> {
    let reply = bus
        .call_sync(
            Some(name),
            PATH,
            IFACE,
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            INTERACTIVE_TIMEOUT_MS,
            gio::Cancellable::NONE,
        )
        .map_err(|e| mechanism_error(&e.to_string(), lang))?;
    let error = string(&first(&reply));
    if !error.is_empty() {
        return Err(cups_refused(&error, lang));
    }
    Ok(reply)
}

/// CUPS's refusal text as the error of a printer write.
pub(crate) fn cups_refused(error: &str, lang: Lang) -> ApiError {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found") || lower.contains("does not exist") {
        return ApiError::tr(
            "not_found",
            lang,
            "printers.notFound",
            crate::i18n::NO_PARAMS,
        );
    }
    ApiError::tr(
        "failed",
        lang,
        "printers.failed",
        &json!({"error": error.chars().take(300).collect::<String>()}),
    )
}

/// `call` that answers `{ok: true}`.
pub(crate) fn ok(
    bus: &gio::DBusConnection,
    name: &str,
    method: &str,
    args: glib::Variant,
    lang: Lang,
) -> ApiResult {
    call(bus, name, method, args, lang).map(|_| json!({"ok": true}))
}

/// DevicesGet(timeout, limit, include, exclude): the scheduler's backends' devices as `a{ss}`.
pub(crate) fn devices(
    bus: &gio::DBusConnection,
    name: &str,
    timeout: i32,
    exclude: &[&str],
    lang: Lang,
) -> Result<serde_json::Map<String, Value>, ApiError> {
    use glib::variant::ToVariant;
    let include: Vec<&str> = Vec::new();
    let reply = call(
        bus,
        name,
        "DevicesGet",
        (timeout, 0i32, include, exclude.to_vec()).to_variant(),
        lang,
    )?;
    let devices = crate::services::system::common::variant_json(&reply.child_value(1));
    Ok(devices.as_object().cloned().unwrap_or_default())
}
