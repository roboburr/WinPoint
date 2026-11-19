//! `input.sources.set`, `input.options.set`, `input.preview` and `input.engineSetup` (batch A,
//! notes/claude/settings-parity.md §5.2, §5.8): the input sources of Taalopties and the special character keys of
//! Geavanceerde toetsenbordinstellingen, written as GNOME Settings writes them.
//!
//! * `input.sources.set {sources: [[type, id]]}` writes `org.gnome.desktop.input-sources sources` (its allow row is
//!   `Service("input.sources.set")`, amendment 50) and then, as Ubuntu's GNOME Settings does, the system keyboard:
//!   `pkexec /usr/libexec/gnome-control-center-set-debian-keyboard <layouts> <variants> [options]` with the xkb
//!   sources' layouts and variants and the current xkb-options (characters `[a-zA-Z0-9,_:-]`). It runs
//!   dpkg-reconfigure, so it runs after the answer, in a thread of its own (amendment 52), and only when polkit
//!   already allows `org.gnome.controlcenter.set-keyboard` without asking (GNOME Settings' GPermission check: YES for
//!   the local sudo users on Ubuntu; anyone else keeps the session setting only, without a password prompt). Its end
//!   is the event `input.changed {what: "systemKeyboard", ok}`.
//! * `input.options.set {lv3?, compose?}` changes only the `lv3:*` and `compose:*` entries of `xkb-options` (row
//!   `Service("input.options.set")`) and keeps every other option: a string sets that option, null removes the kind
//!   (Standaard van de indeling / Uit), an absent argument leaves it. GNOME's "off" for the alternate characters key is
//!   `lv3:ralt_alt`.
//! * `input.preview {layout}` starts tecla (GNOME Settings' View Keyboard Layout) through `tools::system_tool`.
//! * `input.engineSetup {id}` asks GNOME Shell to start the engine's setup (launchBridge.js `launch.ibusSetup`), so
//!   no setup program (several are Python) runs under a WinPoint process.
use super::layouts::engine_id_ok;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::gsettings::{self, Env};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

pub(crate) const SCHEMA: &str = "org.gnome.desktop.input-sources";
const MAX_SOURCES: usize = 64;
pub(crate) const HELPER: &str = "gnome-control-center-set-debian-keyboard";
pub(crate) const KEYBOARD_ACTION: &str = "org.gnome.controlcenter.set-keyboard";

/// GNOME Settings' Alternate Characters Key choices (cc-keyboard-panel.c), `lv3:ralt_alt` being its "off".
pub(crate) const LV3: &[&str] = &[
    "lv3:lalt_switch",
    "lv3:ralt_switch",
    "lv3:lwin_switch",
    "lv3:rwin_switch",
    "lv3:menu_switch",
    "lv3:switch",
    "lv3:ralt_alt",
];
/// GNOME Settings' Compose Key choices.
pub(crate) const COMPOSE: &[&str] = &[
    "compose:ralt",
    "compose:lwin",
    "compose:rwin",
    "compose:menu",
    "compose:lctrl",
    "compose:rctrl",
    "compose:caps",
    "compose:sclk",
    "compose:prsc",
    "compose:ins",
];

fn bad(lang: Lang, key: &str) -> ApiError {
    ApiError::tr("bad_args", lang, key, NO_PARAMS)
}

/// An input source id: xkb `layout[+variant]` or an IBus engine name (§6.6's characters, `@` for m17n).
pub(crate) fn source_ok(kind: &str, id: &str) -> bool {
    match kind {
        "xkb" => {
            let mut parts = id.splitn(2, '+');
            let ok = |p: &str| {
                !p.is_empty()
                    && p.len() <= 64
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            };
            parts.next().is_some_and(ok) && parts.next().is_none_or(ok)
        }
        "ibus" => engine_id_ok(id),
        _ => false,
    }
}

/// The sources of `input.sources.set` (each `[type, id]`, duplicates dropped as GNOME's chooser never adds one).
pub(crate) fn parse_sources(args: &Value, lang: Lang) -> Result<Vec<(String, String)>, ApiError> {
    let list = args["sources"]
        .as_array()
        .filter(|l| l.len() <= MAX_SOURCES)
        .ok_or_else(|| bad(lang, "input.badSources"))?;
    let mut out: Vec<(String, String)> = Vec::with_capacity(list.len());
    for pair in list {
        let (Some(kind), Some(id)) = (pair[0].as_str(), pair[1].as_str()) else {
            return Err(bad(lang, "input.badSources"));
        };
        if pair.as_array().map(Vec::len) != Some(2) || !source_ok(kind, id) {
            return Err(bad(lang, "input.badSources"));
        }
        if !out.iter().any(|(k, i)| k == kind && i == id) {
            out.push((kind.to_owned(), id.to_owned()));
        }
    }
    Ok(out)
}

/// The characters GNOME's helper takes (`[a-zA-Z0-9,_:-]`, its own `case` check).
fn helper_chars(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b',' | b'_' | b':' | b'-'))
}

/// The helper's argv after its path: `<layouts> <variants> [options]` from the xkb sources (in order) and the
/// xkb-options; None when there is no xkb source or a value the helper would refuse.
pub(crate) fn debian_args(sources: &[(String, String)], options: &[String]) -> Option<Vec<String>> {
    let (mut layouts, mut variants) = (Vec::new(), Vec::new());
    for (kind, id) in sources {
        if kind != "xkb" {
            continue;
        }
        let (layout, variant) = id.split_once('+').unwrap_or((id, ""));
        layouts.push(layout.to_owned());
        variants.push(variant.to_owned());
    }
    if layouts.is_empty() {
        return None;
    }
    let mut args = vec![layouts.join(","), variants.join(",")];
    let options = options.join(",");
    if !options.is_empty() {
        args.push(options);
    }
    args.iter().all(|a| helper_chars(a)).then_some(args)
}

/// `xkb-options` with `lv3` and `compose` applied (Some(None): remove that kind; None: keep it).
pub(crate) fn merge_options(
    current: &[String],
    lv3: Option<Option<&str>>,
    compose: Option<Option<&str>>,
) -> Vec<String> {
    let mut out: Vec<String> = current
        .iter()
        .filter(|o| !(lv3.is_some() && o.starts_with("lv3:")))
        .filter(|o| !(compose.is_some() && o.starts_with("compose:")))
        .cloned()
        .collect();
    for value in [lv3, compose].into_iter().flatten().flatten() {
        out.push(value.to_owned());
    }
    out
}

/// The optional `lv3` / `compose` argument: absent (None), null (Some(None)) or one of `allowed`.
fn option_arg<'a>(
    args: &'a Value,
    name: &str,
    allowed: &[&str],
    lang: Lang,
) -> Result<Option<Option<&'a str>>, ApiError> {
    match args.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(Some(Some(s.as_str()))),
        Some(_) => Err(bad(lang, "input.badOption")),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// `input.sources.set` on `env`: the write, and the helper's argv when the system keyboard should follow.
pub(crate) fn set_sources(
    env: &mut Env,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<(Value, Option<Vec<String>>), ApiError> {
    let sources = parse_sources(args, lang)?;
    let value: Vec<Value> = sources.iter().map(|(k, i)| json!([k, i])).collect();
    let items = gsettings::write_values_in(
        env,
        "input.sources.set",
        SCHEMA,
        None,
        &[("sources", json!(value))],
        lang,
        dry_run,
    )?;
    if dry_run {
        return Ok((items, None));
    }
    let options = gsettings::read_item_in(env, SCHEMA, None, "xkb-options", lang)
        .map(|item| strings(&item["value"]))
        .unwrap_or_default();
    Ok((items, debian_args(&sources, &options)))
}

/// `input.options.set` on `env`.
pub(crate) fn set_options(
    env: &mut Env,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let lv3 = option_arg(args, "lv3", LV3, lang)?;
    let compose = option_arg(args, "compose", COMPOSE, lang)?;
    let current = gsettings::read_item_in(env, SCHEMA, None, "xkb-options", lang)?;
    let merged = merge_options(&strings(&current["value"]), lv3, compose);
    let mut out = gsettings::write_values_in(
        env,
        "input.options.set",
        SCHEMA,
        None,
        &[("xkb-options", json!(merged))],
        lang,
        dry_run,
    )?;
    out["options"] = json!(merged);
    Ok(out)
}

/// This process as a polkit subject (`unix-process`: pid, start time from /proc, uid).
fn subject() -> Option<glib::Variant> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after = stat.get(stat.rfind(')')? + 2..)?;
    // field 22 of proc_pid_stat(5); the fields after the comm start at field 3
    let start: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let mut details: HashMap<String, glib::Variant> = HashMap::new();
    details.insert("pid".into(), std::process::id().to_variant());
    details.insert("start-time".into(), start.to_variant());
    details.insert(
        "uid".into(),
        (unsafe { libc::getuid() } as i32).to_variant(),
    );
    Some(("unix-process", details).to_variant())
}

/// Whether polkit (at `destination` on `bus`) allows `action` for this process without asking: GNOME Settings'
/// GPermission `allowed`. Any error (no polkit, a timeout) is no.
pub(crate) fn allowed_without_prompt(
    bus: &gio::DBusConnection,
    destination: &str,
    action: &str,
) -> bool {
    let Some(subject) = subject() else {
        return false;
    };
    let details: HashMap<String, String> = HashMap::new();
    let args = glib::Variant::tuple_from_iter([
        subject,
        action.to_variant(),
        details.to_variant(),
        0u32.to_variant(),
        "".to_variant(),
    ]);
    bus.call_sync(
        Some(destination),
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
        "CheckAuthorization",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .ok()
    .and_then(|reply| reply.child_value(0).child_value(0).get::<bool>())
    .unwrap_or(false)
}

/// Run GNOME's helper through pkexec and wait for it: true when it succeeded.
pub(crate) fn run_helper(pkexec: &Path, helper: &Path, args: &[String]) -> bool {
    std::process::Command::new(pkexec)
        .arg(helper)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The system keyboard after a sources write (module docs), in a thread of its own; `done(ok)` at its end, not at
/// all when polkit would ask (no prompt) or pkexec is not there.
pub(crate) fn sync_system_keyboard(args: Vec<String>, done: impl FnOnce(bool) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("win11-set-keyboard".into())
        .spawn(move || {
            let (Some(pkexec), Some(helper)) = (
                crate::tools::system_tool("pkexec"),
                crate::tools::libexec(HELPER),
            ) else {
                return;
            };
            if !pkexec.is_file() {
                return;
            }
            let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else {
                return;
            };
            if !allowed_without_prompt(&bus, "org.freedesktop.PolicyKit1", KEYBOARD_ACTION) {
                return;
            }
            done(run_helper(&pkexec, &helper, &args));
        });
    drop(spawned);
}

/// A layout tecla shows: `layout` or `layout+variant`.
pub(crate) fn preview_ok(layout: &str) -> bool {
    source_ok("xkb", layout)
}

/// `input.preview {layout}`: tecla for that layout, detached (a thread reaps it).
pub(crate) fn preview(args: &Value, lang: Lang) -> ApiResult {
    let layout = args["layout"].as_str().unwrap_or("");
    if !preview_ok(layout) {
        return Err(bad(lang, "input.badLayout"));
    }
    let Some(tecla) = crate::tools::system_tool("tecla").filter(|p| p.is_file()) else {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "input.noTecla",
            NO_PARAMS,
        ));
    };
    let mut child = std::process::Command::new(tecla)
        .arg(layout)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ApiError::tr("failed", lang, "input.previewFailed", NO_PARAMS))?;
    let _ = std::thread::Builder::new()
        .name("win11-tecla".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(json!({ "started": true }))
}

/// `input.engineSetup {id}` through the bridge's `launch.ibusSetup`.
pub(crate) fn engine_setup(
    args: &Value,
    lang: Lang,
    bridge: &dyn Fn(&str, &Value) -> Result<ApiResult, String>,
) -> ApiResult {
    let id = args["id"].as_str().unwrap_or("");
    if !engine_id_ok(id) {
        return Err(bad(lang, "input.badEngine"));
    }
    match bridge("launch.ibusSetup", &json!({ "engine": id })) {
        Ok(result) => result,
        Err(_) => Err(ApiError::tr(
            "unsupported",
            lang,
            "input.shellUnavailable",
            NO_PARAMS,
        )),
    }
}

fn through_shell(result: Result<ApiResult, String>, lang: Lang) -> ApiResult {
    result.unwrap_or_else(|_| {
        Err(ApiError::tr(
            "unsupported",
            lang,
            "input.shellUnavailable",
            NO_PARAMS,
        ))
    })
}

/// `input.sources.current {}`: the session's input sources as GNOME Shell has them (inputBridge.js), for the tray's
/// input indicator (amendment 15): `{sources: [{index, type, id, name, short}], current}`.
pub(crate) fn current(
    lang: Lang,
    bridge: &dyn Fn(&str, &Value) -> Result<ApiResult, String>,
) -> ApiResult {
    through_shell(bridge("input.sources.current", &json!({})), lang)
}

/// `input.sources.activate {index}`: switch to that source, as GNOME's keyboard menu does.
pub(crate) fn activate(
    args: &Value,
    lang: Lang,
    bridge: &dyn Fn(&str, &Value) -> Result<ApiResult, String>,
) -> ApiResult {
    let index = args["index"]
        .as_u64()
        .filter(|i| (*i as usize) < MAX_SOURCES)
        .ok_or_else(|| bad(lang, "input.badSources"))?;
    through_shell(
        bridge("input.sources.activate", &json!({ "index": index })),
        lang,
    )
}
