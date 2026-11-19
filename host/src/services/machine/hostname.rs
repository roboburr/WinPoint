//! `machine.setName`: the computer's name, as GNOME Settings' About panel sets it (cc-hostname.c): the name as typed
//! is hostname1's PrettyHostname, its ASCII form (hostname-helper.c `pretty_hostname_to_static`) the static hostname.
//! hostname1 checks polkit itself (`set-static-hostname`, which implies `set-hostname` and `set-machine-info`; Ubuntu's
//! rules answer yes for a local administrator).
use super::{Buses, HOSTNAME, dry, unavailable};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use glib::variant::ToVariant;
use serde_json::json;

/// GNOME's entry takes 64 characters at most (cc-about-page.ui, the device name row).
pub(crate) const MAX_CHARS: usize = 64;

/// GLib's `g_str_to_ascii(text, "C")`: the transliteration GNOME's helper starts with (é → e, ß → ss, ...).
fn to_ascii(text: &str) -> String {
    let Ok(c) = std::ffi::CString::new(text) else {
        return String::new();
    };
    unsafe {
        let out = glib::ffi::g_str_to_ascii(c.as_ptr(), c"C".as_ptr());
        if out.is_null() {
            return String::new();
        }
        let s = std::ffi::CStr::from_ptr(out).to_string_lossy().into_owned();
        glib::ffi::g_free(out.cast());
        s
    }
}

/// hostname-helper.c `pretty_hostname_to_static(pretty, FALSE)`: ASCII, apostrophes removed, every character other
/// than a letter, a digit or `-` a `-`, no leading, trailing or doubled dashes, lowercase, at most 63 characters.
/// None when nothing is left ("!!!").
pub(crate) fn static_name(pretty: &str) -> Option<String> {
    let ascii = to_ascii(pretty);
    let mut out = String::new();
    for c in ascii.chars().filter(|c| *c != '\'') {
        let c = if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '-'
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c.to_ascii_lowercase());
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.truncate(63);
    while out.ends_with('-') {
        out.pop();
    }
    (!out.is_empty()).then_some(out)
}

/// A name for the computer: trimmed, 1-64 characters, no control characters, with an ASCII form left.
pub(crate) fn clean(name: &str) -> Option<(String, String)> {
    let pretty = name.trim();
    if pretty.is_empty()
        || pretty.chars().count() > MAX_CHARS
        || pretty.chars().any(char::is_control)
    {
        return None;
    }
    Some((pretty.to_owned(), static_name(pretty)?))
}

/// `machine.setName {name}` → `{ok, prettyHostname, staticHostname}`.
pub(crate) fn set_name(b: &Buses, name: &str, lang: Lang, dry_run: bool) -> ApiResult {
    let (pretty, fixed) =
        clean(name).ok_or_else(|| ApiError::tr("bad_args", lang, "machine.badName", NO_PARAMS))?;
    if dry_run {
        return dry();
    }
    let peer = b
        .hostname()
        .ok_or_else(|| unavailable(lang, "machine.hostnameUnavailable"))?;
    let (_, path, iface) = HOSTNAME;
    peer.interactive(
        path,
        iface,
        "SetPrettyHostname",
        &(pretty.as_str(), true).to_variant(),
        lang,
    )?;
    peer.interactive(
        path,
        iface,
        "SetStaticHostname",
        &(fixed.as_str(), true).to_variant(),
        lang,
    )?;
    Ok(json!({"ok": true, "prettyHostname": pretty, "staticHostname": fixed}))
}
