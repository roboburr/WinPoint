//! `input.capture` and `input.capture.cancel` (batch A, notes/claude/settings-parity.md §5.2, amendment 44): the key
//! combination of "Sneltoets instellen", captured by GNOME Shell (extension/inputBridge.js: `Main.pushModal`, one key
//! press, release), so no GTK inhibit prompt and no PermissionStore write.
//!
//! The bridge answers `{keyval, mods}` (a keysym and the modifier names it saw), `{disabled: true}` (a plain
//! Backspace), `{cancelled: true}` (Esc, a click, its timeout or `input.capture.cancel`); this side turns the keysym
//! into GTK's accelerator name through libxkbcommon (`xkb_keysym_get_name` of the lower-case keysym, as
//! `gtk_accelerator_name` writes it), which GNOME Shell itself has no binding for. The call waits for the user:
//! `gnome::call_api_with_timeout`, 5 s longer than the capture's own timeout (default 30 s).
//!
//! An answer of none of these shapes (a bridge that cannot wait yet: panelBridge.js's synchronous `Call` turns the
//! capture's promise into `{}`; integration note in notes/claude/settings-a.md) releases the grab with
//! `input.capture.cancel` and is `unsupported`, so pickers::KeyCapture listens itself.
use crate::dlib::DlError;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
/// How much longer the D-Bus call waits than the capture itself.
const SLACK_MS: u64 = 5_000;

crate::dlib::dlib! {
    /// libxkbcommon 1: keysym names, as GTK and Mutter spell them.
    pub(crate) struct Xkb ["libxkbcommon.so.0"] {
        fn xkb_keysym_get_name(u32, *mut libc::c_char, usize) -> libc::c_int;
        fn xkb_keysym_to_lower(u32) -> u32;
    }
}

static XKB: OnceLock<Result<Xkb, DlError>> = OnceLock::new();

/// The name of a keysym (`a`, `Page_Up`, `XF86AudioMute`), lower-cased first as GTK does; None without
/// libxkbcommon or for a keysym it has no name for.
pub(crate) fn keysym_name(keyval: u32) -> Option<String> {
    let xkb = XKB.get_or_init(Xkb::load).as_ref().ok()?;
    let lower = unsafe { (xkb.xkb_keysym_to_lower)(keyval) };
    let mut buf = [0 as libc::c_char; 64];
    let n = unsafe { (xkb.xkb_keysym_get_name)(lower, buf.as_mut_ptr(), buf.len()) };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    let bytes: Vec<u8> = buf[..n as usize].iter().map(|c| *c as u8).collect();
    let name = String::from_utf8(bytes).ok()?;
    // xkb names a keysym it does not know as 0x...; GTK and Mutter want a real name
    if name.starts_with("0x") {
        return None;
    }
    Some(
        GTK_NAMES
            .iter()
            .find(|(xkb, _)| *xkb == name)
            .map_or(name.clone(), |(_, gtk)| (*gtk).to_owned()),
    )
}

/// Where xkbcommon's first name for a keysym is not the one gdk_keyval_name (and GNOME's defaults) use.
pub(crate) const GTK_NAMES: &[(&str, &str)] = &[
    ("Prior", "Page_Up"),
    ("Next", "Page_Down"),
    ("KP_Prior", "KP_Page_Up"),
    ("KP_Next", "KP_Page_Down"),
];

/// GTK's modifier names, in gtk_accelerator_name's order.
const MODS: &[(&str, &str)] = &[
    ("Shift", "<Shift>"),
    ("Control", "<Control>"),
    ("Alt", "<Alt>"),
    ("Super", "<Super>"),
    ("Hyper", "<Hyper>"),
    ("Meta", "<Meta>"),
];

/// The accelerator of a keysym name and the bridge's modifier names (unknown names are ignored).
pub(crate) fn accel_name(key: &str, mods: &[String]) -> String {
    let mut out = String::new();
    for (name, spelled) in MODS {
        if mods.iter().any(|m| m == name) {
            out.push_str(spelled);
        }
    }
    out.push_str(key);
    out
}

/// The capture's timeout from the arguments (ms).
pub(crate) fn timeout_ms(args: &Value) -> u64 {
    args["timeout"].as_u64().map_or(DEFAULT_TIMEOUT_MS, |t| {
        t.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
    })
}

/// A bridge call with its own D-Bus timeout (crate::gnome::call_api_with_timeout; tests hand in a fake).
pub(crate) type TimedBridge<'a> = &'a dyn Fn(&str, &Value, i32) -> Result<ApiResult, String>;

fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "input.shellUnavailable", NO_PARAMS)
}

/// The host's answer to a bridge answer: `{accel}`, `{disabled}`, `{cancelled}`, or None for any other shape.
pub(crate) fn answer(v: &Value, name: impl Fn(u32) -> Option<String>) -> Option<Value> {
    if v["cancelled"] == true {
        return Some(json!({"cancelled": true}));
    }
    if v["disabled"] == true {
        return Some(json!({"disabled": true}));
    }
    let keyval = u32::try_from(v["keyval"].as_u64()?).ok()?;
    let mods: Vec<String> = v["mods"]
        .as_array()?
        .iter()
        .filter_map(|m| m.as_str().map(str::to_owned))
        .collect();
    let accel = accel_name(&name(keyval)?, &mods);
    super::shortcuts::parse_accel(&accel).map(|_| json!({ "accel": accel }))
}

/// `input.capture {timeout?}`.
pub(crate) fn capture(args: &Value, lang: Lang, bridge: TimedBridge) -> ApiResult {
    let timeout = timeout_ms(args);
    let wait = (timeout + SLACK_MS) as i32;
    match bridge("input.capture", &json!({ "timeout": timeout }), wait) {
        Ok(Ok(v)) => match answer(&v, keysym_name) {
            Some(out) => Ok(out),
            None => {
                // the grab may still be up (a bridge that answered before the key came): end it
                let _ = bridge("input.capture.cancel", &json!({}), 3000);
                Err(unavailable(lang))
            }
        },
        Ok(Err(error)) => Err(error),
        Err(_) => Err(unavailable(lang)),
    }
}

/// `input.capture.cancel {}`: the bridge ends its grab and answers the waiting capture with `{cancelled: true}`.
/// Nothing to end (no extension, no capture) is fine too.
pub(crate) fn cancel(bridge: TimedBridge) -> ApiResult {
    let _ = bridge("input.capture.cancel", &json!({}), 3000);
    Ok(json!({}))
}
