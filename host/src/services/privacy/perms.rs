//! xdg-desktop-portal's PermissionStore on the session bus, the store GNOME Settings' Apps panel and its Privacy
//! pages read and write (cc-applications-panel.c, cc-location-panel.c, cc-camera-panel.c): one (table, id) entry per
//! permission that holds, per portal app id, a list of strings. Only the entries GNOME Settings 50 shows are
//! reachable ([`KINDS`]); "Systeemsneltoetsen blokkeren" is `gnome`/`shortcuts-inhibitor` (amendment 7,
//! inhibitShortcutsDialog.js APP_PERMISSIONS_ID).
use crate::services::system::common::remote_error_name;
use glib::prelude::*;
use serde_json::{Value, json};

pub(super) const NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const IFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const TIMEOUT_MS: i32 = 5000;

/// One permission GNOME Settings shows: its PermissionStore entry and the values it stores.
pub(super) struct Kind {
    /// The name privacy.app uses for it.
    pub key: &'static str,
    pub table: &'static str,
    pub id: &'static str,
    /// What GNOME stores for allowed and for not allowed.
    pub yes: &'static str,
    pub no: &'static str,
    /// Every first value the API accepts.
    pub values: &'static [&'static str],
}

const YES_NO: &[&str] = &["yes", "no", "ask"];

pub(super) const KINDS: &[Kind] = &[
    Kind {
        key: "location",
        table: "location",
        id: "location",
        yes: "EXACT",
        no: "NONE",
        values: &["NONE", "COUNTRY", "CITY", "NEIGHBORHOOD", "STREET", "EXACT"],
    },
    Kind {
        key: "camera",
        table: "devices",
        id: "camera",
        yes: "yes",
        no: "no",
        values: YES_NO,
    },
    Kind {
        key: "microphone",
        table: "devices",
        id: "microphone",
        yes: "yes",
        no: "no",
        values: YES_NO,
    },
    Kind {
        key: "screenshot",
        table: "screenshot",
        id: "screenshot",
        yes: "yes",
        no: "no",
        values: YES_NO,
    },
    Kind {
        key: "wallpaper",
        table: "wallpaper",
        id: "wallpaper",
        yes: "yes",
        no: "no",
        values: YES_NO,
    },
    Kind {
        key: "shortcutsInhibit",
        table: "gnome",
        id: "shortcuts-inhibitor",
        yes: "GRANTED",
        no: "DENIED",
        values: &["GRANTED", "DENIED"],
    },
    Kind {
        key: "background",
        table: "background",
        id: "background",
        yes: "yes",
        no: "no",
        values: YES_NO,
    },
];

/// The permission of a (table, id) pair, if GNOME Settings shows it.
pub(super) fn kind(table: &str, id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.table == table && k.id == id)
}

/// A portal app id: a Flatpak id, `snap.<name>` or a host app's desktop id without `.desktop`.
pub(super) fn app_id_ok(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= 255
        && !app.starts_with('.')
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Whether a stored value allows (GNOME: anything but the "no" value; nothing stored counts as allowed).
pub(super) fn allowed(kind: &Kind, value: Option<&[String]>) -> bool {
    value
        .and_then(<[String]>::first)
        .is_none_or(|first| first != kind.no)
}

/// What to store for `allowed`; location keeps its timestamp, as GNOME's location panel does.
pub(super) fn stored(kind: &Kind, allowed: bool, current: Option<&[String]>) -> Vec<String> {
    let first = if allowed { kind.yes } else { kind.no }.to_owned();
    if kind.table != "location" {
        return vec![first];
    }
    let stamp = current
        .and_then(|c| c.get(1))
        .filter(|t| timestamp_ok(t))
        .cloned()
        .unwrap_or_else(|| "0".into());
    vec![first, stamp]
}

fn timestamp_ok(t: &str) -> bool {
    !t.is_empty() && t.len() <= 20 && t.bytes().all(|b| b.is_ascii_digit())
}

/// The API's `value`: true or false (GNOME's own values), one string, or a list of strings valid for the kind.
pub(super) fn parse_value(
    kind: &Kind,
    value: &Value,
    current: Option<&[String]>,
) -> Option<Vec<String>> {
    match value {
        Value::Bool(on) => Some(stored(kind, *on, current)),
        Value::String(s) => parse_value(kind, &json!([s]), current),
        Value::Array(items) => {
            let list: Vec<String> = items
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<_>>()?;
            let (first, rest) = list.split_first()?;
            let rest_ok = rest.is_empty()
                || (kind.table == "location" && rest.len() == 1 && timestamp_ok(&rest[0]));
            (kind.values.contains(&first.as_str()) && rest_ok).then_some(list)
        }
        _ => None,
    }
}

/// The apps and values of a `Lookup` answer's `a{sas}`.
pub(super) fn parse_lookup(permissions: &glib::Variant) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    if permissions.type_().as_str() != "a{sas}" {
        return out;
    }
    for i in 0..permissions.n_children() {
        let entry = permissions.child_value(i);
        let app = entry.child_value(0).str().unwrap_or("").to_owned();
        let values = entry
            .child_value(1)
            .get::<Vec<String>>()
            .unwrap_or_default();
        out.push((app, values));
    }
    out.sort();
    out
}

/// Why a call failed.
#[derive(Debug, PartialEq)]
pub(super) enum StoreError {
    /// No PermissionStore on this bus (a session without xdg-desktop-portal).
    Missing,
    Failed(String),
}

fn error(e: glib::Error) -> StoreError {
    let text = e.to_string();
    match remote_error_name(&text) {
        Some(name)
            if name.ends_with(".ServiceUnknown")
                || name.ends_with(".NameHasNoOwner")
                || name.ends_with(".UnknownMethod")
                || name.ends_with(".UnknownObject") =>
        {
            StoreError::Missing
        }
        _ => StoreError::Failed(text),
    }
}

fn call(
    bus: &gio::DBusConnection,
    name: &str,
    method: &str,
    args: glib::Variant,
) -> Result<glib::Variant, glib::Error> {
    bus.call_sync(
        Some(name),
        PATH,
        IFACE,
        method,
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        TIMEOUT_MS,
        gio::Cancellable::NONE,
    )
}

/// The entry a PermissionStore `Changed (s table, s id, b deleted, v data, a{sas} permissions)` signal is about,
/// when it is one GNOME Settings shows ([`KINDS`]); any other table (notifications, documents, ...) is None.
pub(super) fn changed_kind(params: &glib::Variant) -> Option<&'static Kind> {
    if params.n_children() < 2 {
        return None;
    }
    let (table, id) = (params.child_value(0), params.child_value(1));
    kind(table.str()?, id.str()?)
}

/// Follow the store's `Changed` signal on `bus` (dispatched in the caller's thread-default main context):
/// `f(kind)` for every change of an entry of [`KINDS`], whoever made it (a portal dialog answered, another
/// program, this host). GNOME Settings' Apps, Location and Camera panels follow the same signal.
pub(super) fn watch(
    bus: &gio::DBusConnection,
    name: &str,
    f: impl Fn(&'static Kind) + 'static,
) -> gio::SignalSubscriptionId {
    bus.signal_subscribe(
        Some(name),
        Some(IFACE),
        Some("Changed"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            if let Some(kind) = changed_kind(params) {
                f(kind);
            }
        },
    )
}

/// `Lookup(table, id)`: every app's value; an entry that does not exist yet is empty.
pub(super) fn lookup(
    bus: &gio::DBusConnection,
    name: &str,
    table: &str,
    id: &str,
) -> Result<Vec<(String, Vec<String>)>, StoreError> {
    match call(bus, name, "Lookup", (table, id).to_variant()) {
        Ok(reply) if reply.n_children() >= 1 => Ok(parse_lookup(&reply.child_value(0))),
        Ok(_) => Ok(Vec::new()),
        Err(e) => {
            let text = e.to_string();
            if remote_error_name(&text).is_some_and(|n| n.ends_with(".NotFound")) {
                Ok(Vec::new())
            } else {
                Err(error(e))
            }
        }
    }
}

/// `SetPermission(table, create, id, app, values)`.
pub(super) fn set(
    bus: &gio::DBusConnection,
    name: &str,
    table: &str,
    id: &str,
    app: &str,
    values: &[String],
) -> Result<(), StoreError> {
    let args = (table, true, id, app, values.to_vec()).to_variant();
    call(bus, name, "SetPermission", args)
        .map(|_| ())
        .map_err(error)
}

/// `DeletePermission(table, id, app)`, or `Delete(table, id)` for the whole entry when `app` is None.
pub(super) fn delete(
    bus: &gio::DBusConnection,
    name: &str,
    table: &str,
    id: &str,
    app: Option<&str>,
) -> Result<(), StoreError> {
    let result = match app {
        Some(app) => call(bus, name, "DeletePermission", (table, id, app).to_variant()),
        None => call(bus, name, "Delete", (table, id).to_variant()),
    };
    match result {
        Ok(_) => Ok(()),
        Err(e) if remote_error_name(&e.to_string()).is_some_and(|n| n.ends_with(".NotFound")) => {
            Ok(())
        }
        Err(e) => Err(error(e)),
    }
}

/// One app's permission rows for privacy.app: every kind but background, with whether it is stored (GNOME shows a
/// row only for a permission the app asked for) and whether it allows.
pub(super) fn app_rows(
    entries: &[(&'static Kind, Vec<(String, Vec<String>)>)],
    app: &str,
) -> Vec<Value> {
    entries
        .iter()
        .filter(|(kind, _)| kind.key != "background")
        .map(|(kind, apps)| {
            let value = apps
                .iter()
                .find(|(a, _)| a == app)
                .map(|(_, v)| v.as_slice());
            json!({
                "key": kind.key,
                "table": kind.table,
                "id": kind.id,
                "set": value.is_some(),
                "allowed": allowed(kind, value),
                "value": value.map(<[String]>::to_vec),
            })
        })
        .collect()
}
