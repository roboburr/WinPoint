//! The Bluetooth adapter as GNOME Settings shows it (cc-bluetooth-panel.c, gnome-bluetooth's settings widget): its
//! name for "Detecteerbaar als" (BlueZ `Alias`, which follows the device name of About), discoverable while the
//! page shows, and "Bestanden verzenden" through GNOME's own `bluetooth-sendto --device --name`.
//!
//! Discoverable is a lease: the page asks for it when it opens and renews it every minute; the host turns it off
//! again when the page goes or its window closes (`{on: false}`), when [`LEASE`] passes without a renewal (a page
//! that stopped renewing while its backend lives on: devices.rs arms a one-shot source at the lease's end) and when
//! the host stops (`Devices::shutdown`, which needs the host's SIGTERM handler: note 13 for I in
//! notes/claude/settings-b.md). Only a Discoverable the lease turned on is turned off: an adapter that was
//! discoverable already (bluetoothctl) stays as it was. BlueZ's own DiscoverableTimeout stays as it is; when it ends
//! the visibility first, the next renewal turns it on again.
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use crate::services::system::common::{call, first, setprop, string, truth};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ADAPTER: &str = "org.bluez.Adapter1";
/// How long a lease lasts without a renewal (the page renews every 60 s).
pub(super) const LEASE: Duration = Duration::from_secs(150);
/// The longest name BlueZ gives a device (248 bytes of UTF-8).
const NAME_MAX: usize = 248;

/// The discoverable lease (module docs).
#[derive(Default)]
pub(super) struct Lease {
    until: Option<Instant>,
    /// The adapter it holds, and whether the lease turned Discoverable on there.
    path: Option<String>,
    ours: bool,
}

impl Lease {
    pub(super) fn active(&self) -> bool {
        self.until.is_some()
    }

    pub(super) fn expired(&self) -> bool {
        self.until.is_some_and(|at| Instant::now() >= at)
    }

    /// When the lease ends without a renewal (None: no lease).
    pub(super) fn until(&self) -> Option<Instant> {
        self.until
    }
}

pub(super) fn no_bluetooth(lang: Lang) -> ApiError {
    ApiError::tr(
        "unavailable",
        lang,
        "devices.noBluetooth",
        crate::i18n::NO_PARAMS,
    )
}

/// `{available: false}`: no BlueZ, or no adapter.
pub(super) fn absent() -> Value {
    json!({"available": false, "powered": false, "discoverable": false, "alias": "", "address": ""})
}

/// The first adapter of BlueZ's objects (the one system/bluetooth.rs uses too: the lowest path).
pub(super) fn first_adapter(objects: &Value) -> Option<(String, Value)> {
    let mut paths: Vec<(&String, &Value)> = objects
        .as_object()?
        .iter()
        .filter(|(_, v)| v[ADAPTER].is_object())
        .collect();
    paths.sort_by_key(|(k, _)| *k);
    paths
        .first()
        .map(|(k, v)| (k.to_string(), v[ADAPTER].clone()))
}

/// The adapter's answer: its path, address, name ("Detecteerbaar als": Alias, else Name), power and visibility.
pub(super) fn adapter_json(path: &str, props: &Value) -> Value {
    let name = string(&props["Name"]);
    let alias = Some(string(&props["Alias"]))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| name.clone());
    json!({
        "available": true,
        "path": path,
        "address": string(&props["Address"]).to_uppercase(),
        "alias": alias,
        "name": name,
        "powered": truth(&props["Powered"]),
        "discoverable": truth(&props["Discoverable"]),
        "discoverableTimeout": props["DiscoverableTimeout"].as_u64().unwrap_or(0),
        "pairable": truth(&props["Pairable"]),
    })
}

fn objects(bus: &gio::DBusConnection, name: &str) -> Result<Value, String> {
    call(
        bus,
        name,
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        2000,
    )
    .map(|v| first(&v))
}

fn read(bus: &gio::DBusConnection, name: &str) -> Option<(String, Value)> {
    objects(bus, name).ok().and_then(|o| first_adapter(&o))
}

/// `devices.bluetooth.adapter`.
pub(super) fn adapter(bus: &gio::DBusConnection, name: &str) -> Value {
    match read(bus, name) {
        Some((path, props)) => adapter_json(&path, &props),
        None => absent(),
    }
}

pub(super) fn on_arg(v: &Value, lang: Lang) -> Result<bool, ApiError> {
    v.as_bool()
        .ok_or_else(|| super::bad(lang, "devices.badArgs"))
}

/// `devices.bluetooth.setDiscoverable {on}` (module docs): the adapter's answer afterwards. `span` is how long the
/// lease lasts without a renewal ([`LEASE`]; shorter in the tests).
pub(super) fn set_discoverable(
    bus: &gio::DBusConnection,
    name: &str,
    lease: &mut Lease,
    on: bool,
    span: Duration,
    lang: Lang,
) -> ApiResult {
    if !on {
        end_lease(bus, name, lease);
        return Ok(adapter(bus, name));
    }
    let Some((path, props)) = read(bus, name) else {
        return Err(no_bluetooth(lang));
    };
    if lease.path.as_deref().is_some_and(|p| p != path) {
        // another adapter than the one the lease holds: let that one go first
        end_lease(bus, name, lease);
    }
    lease.until = Some(Instant::now() + span);
    lease.path = Some(path.clone());
    if !truth(&props["Powered"]) || truth(&props["Discoverable"]) {
        // off: the renewal after Bluetooth is turned on makes it discoverable; on already: nothing to do
        return Ok(adapter_json(&path, &props));
    }
    setprop(
        bus,
        name,
        &path,
        ADAPTER,
        "Discoverable",
        true.to_variant(),
        3000,
    )
    .map_err(|_| {
        ApiError::tr(
            "failed",
            lang,
            "devices.discoverableFailed",
            crate::i18n::NO_PARAMS,
        )
    })?;
    lease.ours = true;
    Ok(adapter(bus, name))
}

/// Give the lease up: Discoverable off again when the lease turned it on.
pub(super) fn end_lease(bus: &gio::DBusConnection, name: &str, lease: &mut Lease) {
    let held = std::mem::take(lease);
    if held.ours
        && let Some(path) = held.path
    {
        let _ = setprop(
            bus,
            name,
            &path,
            ADAPTER,
            "Discoverable",
            false.to_variant(),
            3000,
        );
    }
}

/// A Bluetooth address: six pairs of hex digits with ':' (upper case).
pub(super) fn address_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A device name as an argument of bluetooth-sendto: no control characters, at most [`NAME_MAX`] bytes.
pub(super) fn clean_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > NAME_MAX {
            break;
        }
        out.push(c);
    }
    out
}

/// The arguments of `devices.bluetooth.sendFiles {address, name}`.
pub(super) fn send_args(args: &Value, lang: Lang) -> Result<(String, String), ApiError> {
    let address = args["address"]
        .as_str()
        .filter(|a| address_ok(a))
        .ok_or_else(|| super::bad(lang, "devices.badDevice"))?
        .to_uppercase();
    let name = match &args["name"] {
        Value::Null => String::new(),
        Value::String(s) => clean_name(s),
        _ => return Err(super::bad(lang, "devices.badArgs")),
    };
    Ok((address, name))
}

/// GNOME's "Send Files…" (cc-bluetooth-panel.c): `bluetooth-sendto --device=<address> --name=<name>`, a GTK 4 window
/// of gnome-bluetooth that picks the files and sends them through obexd. `tool` is `tools::system_tool`'s path.
/// It runs on its own (its window is the user's); a thread reaps it.
pub(super) fn send_files(
    tool: Option<PathBuf>,
    address: &str,
    name: &str,
    lang: Lang,
) -> ApiResult {
    let missing = || {
        ApiError::tr(
            "unsupported",
            lang,
            "devices.sendFilesMissing",
            crate::i18n::NO_PARAMS,
        )
    };
    let tool = tool
        .filter(|p| Path::new(p).is_file())
        .ok_or_else(missing)?;
    let mut command = std::process::Command::new(&tool);
    command.arg(format!("--device={address}"));
    if !name.is_empty() {
        command.arg(format!("--name={name}"));
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| {
            ApiError::tr(
                "failed",
                lang,
                "devices.sendFilesFailed",
                crate::i18n::NO_PARAMS,
            )
        })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(json!({"ok": true}))
}
