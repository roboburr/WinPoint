//! D-Bus and process primitives shared by native system, display and MPRIS services.
use glib::variant::ToVariant;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(crate) fn session() -> Result<gio::DBusConnection, String> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| e.to_string())
}
pub(crate) fn system() -> Result<gio::DBusConnection, String> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(|e| e.to_string())
}
pub(crate) fn call(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&glib::Variant>,
    timeout: i32,
) -> Result<glib::Variant, String> {
    bus.call_sync(
        Some(name),
        path,
        iface,
        method,
        args,
        None,
        gio::DBusCallFlags::NONE,
        timeout,
        gio::Cancellable::NONE,
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn getall(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
) -> Result<Value, String> {
    let v = call(
        bus,
        name,
        path,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(iface,).to_variant()),
        1500,
    )?;
    Ok(variant_json(&v.child_value(0)))
}
pub(crate) fn setprop(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
    prop: &str,
    value: glib::Variant,
    timeout: i32,
) -> Result<(), String> {
    call(
        bus,
        name,
        path,
        "org.freedesktop.DBus.Properties",
        "Set",
        Some(&(iface, prop, value).to_variant()),
        timeout,
    )
    .map(|_| ())
}
/// How long a call that may show GNOME's password dialog waits: the user types the password meanwhile.
pub(crate) const INTERACTIVE_TIMEOUT_MS: i32 = 600_000;
/// A call to a system daemon that checks polkit itself (notes/claude/settings-parity.md §5.8): GNOME Shell's polkit
/// agent shows the password dialog where the policy asks for one (ALLOW_INTERACTIVE_AUTHORIZATION, up to 10 minutes),
/// as GNOME Settings does. A daemon method with its own `interactive` argument gets `true` from the caller. A refusal
/// is `denied core.notAuthorized`, a dismissed dialog `denied core.authCancelled`, anything else `failed
/// core.callFailed` with the daemon's message.
#[allow(dead_code)] // the batches' system writes (hostname1, timedate1, AccountsService, NetworkManager, ...)
pub(crate) fn call_interactive(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&glib::Variant>,
    lang: crate::i18n::Lang,
) -> Result<glib::Variant, crate::files::ApiError> {
    bus.call_sync(
        Some(name),
        path,
        iface,
        method,
        args,
        None,
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        INTERACTIVE_TIMEOUT_MS,
        gio::Cancellable::NONE,
    )
    .map_err(|error| interactive_error(&error.to_string(), lang))
}
/// The D-Bus error name in a GDBus error text ("GDBus.Error:<name>: <message>"), if it has one.
pub(crate) fn remote_error_name(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("GDBus.Error:")?;
    let name = rest.split(':').next()?.trim();
    (!name.is_empty() && !name.contains(' ')).then_some(name)
}
/// The ApiError of a failed interactive call (see [`call_interactive`]).
pub(crate) fn interactive_error(text: &str, lang: crate::i18n::Lang) -> crate::files::ApiError {
    use crate::files::ApiError;
    let name = remote_error_name(text).unwrap_or("").to_ascii_lowercase();
    let lower = text.to_ascii_lowercase();
    if name.ends_with(".cancelled")
        || name.ends_with(".canceled")
        || lower.contains("dismissed")
        || lower.contains("cancelled by user")
    {
        return ApiError::tr("denied", lang, "core.authCancelled", crate::i18n::NO_PARAMS);
    }
    if [
        "notauthorized",
        "accessdenied",
        "permissiondenied",
        "interactiveauthorizationrequired",
        "authfailed",
    ]
    .iter()
    .any(|needle| name.contains(needle))
    {
        return ApiError::tr("denied", lang, "core.notAuthorized", crate::i18n::NO_PARAMS);
    }
    let message = text.strip_prefix("GDBus.Error:").map_or(text, |rest| {
        rest.split_once(": ").map_or(rest, |(_, message)| message)
    });
    ApiError::tr(
        "failed",
        lang,
        "core.callFailed",
        &json!({"error": message.chars().take(300).collect::<String>()}),
    )
}
pub(crate) fn variant_json(v: &glib::Variant) -> Value {
    match v.type_().as_str().as_bytes()[0] {
        b'v' => v
            .as_variant()
            .map(|v| variant_json(&v))
            .unwrap_or(Value::Null),
        b'b' => json!(v.get::<bool>().unwrap_or(false)),
        b'y' => json!(v.get::<u8>().unwrap_or(0)),
        b'n' => json!(v.get::<i16>().unwrap_or(0)),
        b'q' => json!(v.get::<u16>().unwrap_or(0)),
        b'i' => json!(v.get::<i32>().unwrap_or(0)),
        b'u' => json!(v.get::<u32>().unwrap_or(0)),
        b'x' => json!(v.get::<i64>().unwrap_or(0)),
        b't' => json!(v.get::<u64>().unwrap_or(0)),
        b'd' => json!(v.get::<f64>().unwrap_or(0.)),
        b's' | b'o' | b'g' => json!(v.str().unwrap_or("")),
        b'a' if v.type_().as_str().starts_with("a{") => {
            let mut map = Map::new();
            for i in 0..v.n_children() {
                let e = v.child_value(i);
                let key = e.child_value(0);
                if let Some(k) = key.str() {
                    map.insert(k.to_owned(), variant_json(&e.child_value(1)));
                }
            }
            Value::Object(map)
        }
        b'a' | b'(' => Value::Array(
            (0..v.n_children())
                .map(|i| variant_json(&v.child_value(i)))
                .collect(),
        ),
        b'm' => {
            if v.n_children() > 0 {
                variant_json(&v.child_value(0))
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}
pub(crate) fn first(v: &glib::Variant) -> Value {
    if v.n_children() > 0 {
        variant_json(&v.child_value(0))
    } else {
        Value::Null
    }
}
pub(crate) fn object_path(p: &str) -> Result<glib::variant::ObjectPath, String> {
    glib::variant::ObjectPath::try_from(p).map_err(|e| e.to_string())
}
pub(crate) fn dictionary(v: &glib::Variant) -> BTreeMap<String, glib::Variant> {
    let mut out = BTreeMap::new();
    for i in 0..v.n_children() {
        let e = v.child_value(i);
        if let Some(k) = e.child_value(0).str() {
            if let Some(value) = e.child_value(1).as_variant() {
                out.insert(k.to_owned(), value);
            }
        }
    }
    out
}
pub(crate) fn string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Array(v) => v
            .iter()
            .filter(|v| truth(v))
            .map(string)
            .collect::<Vec<_>>()
            .join(", "),
        _ => v.to_string(),
    }
}
pub(crate) fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
pub(crate) fn bool_arg(v: &Value) -> bool {
    if let Some(s) = v.as_str() {
        matches!(s.to_lowercase().as_str(), "1" | "true" | "yes" | "on")
    } else {
        truth(v)
    }
}
pub(crate) fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.trim().parse().ok())
        .or_else(|| v.as_bool().map(|b| if b { 1. } else { 0. }))
}
/// Python int() coercion for JSON scalar API arguments, bounded to native IDs.
pub(crate) fn pyint(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|n| i64::try_from(n).ok()))
            .or_else(|| {
                n.as_f64()
                    .filter(|v| v.is_finite() && *v >= i64::MIN as f64 && *v < i64::MAX as f64)
                    .map(|v| v.trunc() as i64)
            }),
        Value::String(text) => {
            let text = text.trim();
            let (sign, digits) = match text.as_bytes().first() {
                Some(b'+') => ("", &text[1..]),
                Some(b'-') => ("-", &text[1..]),
                _ => ("", text),
            };
            let chars = digits.chars().collect::<Vec<_>>();
            if chars.is_empty() {
                return None;
            }
            let digit = |c: char| unsafe { glib::ffi::g_unichar_digit_value(c as u32) };
            let mut normalized = String::from(sign);
            for (i, c) in chars.iter().copied().enumerate() {
                let n = digit(c);
                if n >= 0 {
                    normalized.push(char::from(b'0' + n as u8));
                } else if c == '_'
                    && i > 0
                    && i + 1 < chars.len()
                    && digit(chars[i - 1]) >= 0
                    && digit(chars[i + 1]) >= 0
                {
                } else {
                    return None;
                }
            }
            normalized.parse().ok()
        }
        _ => None,
    }
}
pub(crate) fn integer(v: &Value) -> Option<i64> {
    number(v).filter(|n| n.is_finite()).map(|n| n as i64)
}
pub(crate) fn round(v: f64, n: i32) -> f64 {
    // Decimal formatting rounds the original binary float, as Python round does.
    // Multiplication by 10^n can create a new exact .5 and give a different tie.
    if n >= 0 {
        format!("{:.*}", n as usize, v).parse().unwrap_or(v)
    } else {
        let m = 10f64.powi(-n);
        (v / m).round_ties_even() * m
    }
}
pub(crate) fn available(executable: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|dir| {
            std::fs::metadata(dir.join(executable)).is_ok_and(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            })
        })
    })
}
pub(crate) struct Output {
    pub ok: bool,
    pub out: String,
    pub err: String,
}
pub(crate) fn run(args: &[&str], seconds: u64, c_locale: bool) -> Output {
    let mut command = Command::new(args.first().copied().unwrap_or(""));
    command
        .args(args.iter().skip(1))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if c_locale {
        command.env("LC_ALL", "C").env("LANG", "C");
    }
    let Ok(mut child) = command.spawn() else {
        return Output {
            ok: false,
            out: String::new(),
            err: "command unavailable".into(),
        };
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read = |r: Option<std::process::ChildStdout>| {
        let mut data = Vec::new();
        if let Some(r) = r {
            let _ = r.take(16 * 1024 * 1024).read_to_end(&mut data);
        }
        String::from_utf8_lossy(&data).into_owned()
    };
    let out = thread::spawn(move || read(stdout));
    let err = thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(r) = stderr {
            let _ = r.take(1024 * 1024).read_to_end(&mut data);
        }
        String::from_utf8_lossy(&data).into_owned()
    });
    let end = Instant::now() + Duration::from_secs(seconds);
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) => {
                if Instant::now() >= end {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    Output {
        ok,
        out: out.join().unwrap_or_default(),
        err: err.join().unwrap_or_default(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variants_preserve_nested_dictionary_types() {
        let v = BTreeMap::from([
            ("volume".to_owned(), 42u32.to_variant()),
            ("label".to_owned(), "a:b".to_variant()),
        ])
        .to_variant();
        assert_eq!(variant_json(&v), json!({"volume":42,"label":"a:b"}));
    }
    #[test]
    fn python_rounding() {
        assert_eq!(round(12.5, 0), 12.);
        assert_eq!(round(13.5, 0), 14.);
        assert_eq!(round(12.345, 2), 12.35);
        assert_eq!(round(2.675, 2), 2.67);
    }
}

#[cfg(test)]
mod int_tests {
    use super::*;
    #[test]
    fn python_id_coercion() {
        assert_eq!(pyint(&json!("  +١_٢٣ ")), Some(123));
        assert_eq!(pyint(&json!("1.5")), None);
        assert_eq!(pyint(&json!(1.5)), Some(1));
        assert_eq!(pyint(&json!(true)), Some(1));
        assert_eq!(pyint(&json!("_123")), None);
        assert_eq!(pyint(&json!("1__2")), None);
    }
}

/// A ServiceContext whose own settings file says `language` (R2-A's en render tests: the services render with
/// `ctx.lang()`, so a test needs its own config dir, not the process's). Removed again when dropped.
#[cfg(test)]
pub(crate) struct LangContext {
    pub root: std::path::PathBuf,
    pub ctx: std::sync::Arc<crate::services::ServiceContext>,
}
#[cfg(test)]
impl LangContext {
    pub fn new(language: &str, kind: &str) -> Self {
        let root = std::env::temp_dir().join(format!("win11-r2a-{}", crate::events::nonce()));
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("settings.json"),
            json!({"language":language}).to_string(),
        )
        .unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = crate::services::ServiceContext::new(
            events,
            config,
            root.join("cache"),
            root.clone(),
            kind.into(),
            false,
        )
        .unwrap();
        Self { root, ctx }
    }
}
#[cfg(test)]
impl Drop for LangContext {
    fn drop(&mut self) {
        self.ctx.events.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
/// The English half of native Settings' own `t("<nl>", "<en>")` pair for `nl` in
/// native/apps/settings/src/pages/display.rs: the D-G2 rows of the host must equal the page's text (R2-A fix round,
/// review m4). The page must be there (the test needs the whole repository) and say `nl`, with one English text.
#[cfg(test)]
pub(crate) fn native_display_text(nl: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../native/apps/settings/src/pages/display.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let quoted = format!("\"{nl}\"");
    let mut english: Vec<&str> = source
        .match_indices(quoted.as_str())
        .filter_map(|(at, _)| {
            // t("<nl>", "<en>"), also over two lines: a comma, then the English literal
            let rest = source[at + quoted.len()..].trim_start().strip_prefix(',')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            rest.split_once('"').map(|(en, _)| en)
        })
        .collect();
    english.dedup();
    assert_eq!(
        english.len(),
        1,
        "display.rs t({quoted}, …): one English text, found {english:?}"
    );
    english[0].to_owned()
}

#[cfg(test)]
mod interactive_tests {
    use super::*;
    use crate::i18n::Lang;
    use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering, mpsc};

    #[test]
    fn interactive_errors_map_to_denied_cancelled_or_failed() {
        let key = |text: &str| interactive_error(text, Lang::Nl).key.unwrap();
        for text in [
            "GDBus.Error:org.freedesktop.PolicyKit1.Error.NotAuthorized: no",
            "GDBus.Error:org.freedesktop.DBus.Error.AccessDenied: Access denied",
            "GDBus.Error:org.freedesktop.Accounts.Error.PermissionDenied: Not authorized",
            "GDBus.Error:org.freedesktop.NetworkManager.PermissionDenied: Insufficient privileges",
            "GDBus.Error:org.freedesktop.DBus.Error.InteractiveAuthorizationRequired: Interactive authentication required.",
        ] {
            assert_eq!(key(text), "core.notAuthorized", "{text}");
        }
        assert_eq!(
            key(
                "GDBus.Error:org.freedesktop.PolicyKit1.Error.Cancelled: Authentication dialog was dismissed by the user"
            ),
            "core.authCancelled"
        );
        let other = interactive_error(
            "GDBus.Error:org.example.Error.Boom: the disk is full",
            Lang::En,
        );
        assert_eq!(
            (other.code.as_str(), other.key.as_deref()),
            ("failed", Some("core.callFailed"))
        );
        assert_eq!(other.message, "That didn’t work: the disk is full");
        assert_eq!(remote_error_name("Timeout was reached"), None);
    }

    /// A private session bus only: tests/rust-sandbox/run-tests.sh's (never the live one under /run/user).
    fn private_bus_address() -> Option<String> {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
        (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
            && !address.contains("/run/user/"))
        .then_some(address)
    }

    const XML: &str = r#"<node><interface name="io.github.win11shell.Test">
        <method name="Ok"><arg type="s" direction="out"/></method>
        <method name="Deny"/><method name="Cancel"/><method name="Boom"/>
    </interface></node>"#;

    #[test]
    fn call_interactive_asks_for_the_password_dialog_and_maps_the_answer() {
        let Some(address) = private_bus_address() else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let flags: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (seen, thread_stop) = (flags.clone(), stop.clone());
        let server = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    gio::Cancellable::NONE,
                )
                .unwrap();
                let node = gio::DBusNodeInfo::for_xml(XML).unwrap();
                let iface = node.lookup_interface("io.github.win11shell.Test").unwrap();
                let id = connection
                    .register_object(
                        "/io/github/win11shell/Test",
                        &iface,
                        move |_, _, _, _, method, _, invocation| {
                            let interactive = invocation
                                .message()
                                .flags()
                                .contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION);
                            seen.lock().unwrap().push(interactive);
                            match method {
                                "Ok" => invocation.return_value(Some(&("done",).to_variant())),
                                "Deny" => invocation.return_dbus_error(
                                    "org.freedesktop.PolicyKit1.Error.NotAuthorized",
                                    "no",
                                ),
                                "Cancel" => invocation.return_dbus_error(
                                    "org.freedesktop.PolicyKit1.Error.Cancelled",
                                    "Authentication dialog was dismissed by the user",
                                ),
                                _ => invocation.return_dbus_error("org.example.Error.Boom", "boom"),
                            }
                        },
                        |_, _, _, _, _| ().to_variant(),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                ready
                    .send((
                        connection.unique_name().unwrap().to_string(),
                        context.clone(),
                    ))
                    .unwrap();
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                connection.unregister_object(id).unwrap();
            });
        });
        let (name, context) = started.recv().unwrap();
        let bus = session().unwrap();
        let path = "/io/github/win11shell/Test";
        let iface = "io.github.win11shell.Test";
        let ok = call_interactive(&bus, &name, path, iface, "Ok", None, Lang::Nl).unwrap();
        assert_eq!(ok.child_value(0).str(), Some("done"));
        let deny = call_interactive(&bus, &name, path, iface, "Deny", None, Lang::Nl).unwrap_err();
        assert_eq!(
            (deny.code.as_str(), deny.key.as_deref()),
            ("denied", Some("core.notAuthorized"))
        );
        let cancel =
            call_interactive(&bus, &name, path, iface, "Cancel", None, Lang::Nl).unwrap_err();
        assert_eq!(cancel.key.as_deref(), Some("core.authCancelled"));
        let boom = call_interactive(&bus, &name, path, iface, "Boom", None, Lang::En).unwrap_err();
        assert_eq!(boom.message, "That didn’t work: boom");
        assert_eq!(
            *flags.lock().unwrap(),
            vec![true; 4],
            "every call allows the password dialog"
        );
        stop.store(true, Ordering::Release);
        context.wakeup();
        server.join().unwrap();
    }
}
