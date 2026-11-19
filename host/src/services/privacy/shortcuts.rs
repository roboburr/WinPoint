//! An app's global shortcuts (GNOME 50, the portal's GlobalShortcuts): gnome-settings-daemon's
//! `org.gnome.settings-daemon.global-shortcuts applications` lists the apps that registered some, the relocatable
//! `.application` schema at `/org/gnome/settings-daemon/global-shortcuts/<app>/` holds their `shortcuts`
//! (`a(sa{sv})`, which the gsettings value mapper refuses, so it is read here natively and is in no allow table,
//! amendment 43). Changes go to xdg-desktop-portal-gnome's `org.gnome.GlobalShortcutsRebind.RebindShortcuts(s,
//! a(sa{sv}))` at `/org/gnome/globalshortcuts`, as GNOME Settings' Apps panel sends them.
use crate::services::system::common::variant_json;
use gio::prelude::*;
use serde_json::{Value, json};

const LIST_SCHEMA: &str = "org.gnome.settings-daemon.global-shortcuts";
const APP_SCHEMA: &str = "org.gnome.settings-daemon.global-shortcuts.application";
/// The bus name of xdg-desktop-portal-gnome, which serves the rebind interface (gnome-control-center's proxy: g-name
/// org.freedesktop.impl.portal.desktop.gnome, g-interface-name org.gnome.GlobalShortcutsRebind).
pub(super) const REBIND_NAME: &str = "org.freedesktop.impl.portal.desktop.gnome";
const REBIND_PATH: &str = "/org/gnome/globalshortcuts";
const REBIND_IFACE: &str = "org.gnome.GlobalShortcutsRebind";

/// One registered action: its id and the portal's description dictionary (`a{sv}`).
#[derive(Clone, Debug)]
pub(super) struct Shortcut {
    pub id: String,
    pub dict: glib::Variant,
}

fn schema(id: &str) -> Option<gio::SettingsSchema> {
    gio::SettingsSchemaSource::default()?.lookup(id, true)
}

/// The settings path of an app's shortcuts, for a valid portal app id.
pub(super) fn path(app: &str) -> Option<String> {
    super::perms::app_id_ok(app)
        .then(|| format!("/org/gnome/settings-daemon/global-shortcuts/{app}/"))
}

/// The first of `candidates` that registered global shortcuts.
pub(super) fn registered(candidates: &[String]) -> Option<String> {
    schema(LIST_SCHEMA)?;
    let list = gio::Settings::new(LIST_SCHEMA).strv("applications");
    candidates
        .iter()
        .find(|c| list.iter().any(|a| a.as_str() == c.as_str()))
        .cloned()
}

/// The shortcuts `app` registered (None when the schema is not installed).
pub(super) fn read(app: &str) -> Option<Vec<Shortcut>> {
    let path = path(app)?;
    schema(APP_SCHEMA)?;
    let value = gio::Settings::with_path(APP_SCHEMA, &path).value("shortcuts");
    Some(parse(&value))
}

/// The entries of an `a(sa{sv})` value.
pub(super) fn parse(value: &glib::Variant) -> Vec<Shortcut> {
    if value.type_().as_str() != "a(sa{sv})" {
        return Vec::new();
    }
    (0..value.n_children())
        .map(|i| value.child_value(i))
        .map(|e| Shortcut {
            id: e.child_value(0).str().unwrap_or("").to_owned(),
            dict: e.child_value(1),
        })
        .collect()
}

fn lookup(dict: &glib::Variant, key: &str) -> Option<glib::Variant> {
    glib::VariantDict::new(Some(dict)).lookup_value(key, None)
}

/// The accelerators of an entry: its `shortcuts` list, else the portal's trigger strings.
pub(super) fn accels(dict: &glib::Variant) -> Vec<String> {
    if let Some(list) = lookup(dict, "shortcuts").and_then(|v| v.get::<Vec<String>>()) {
        return list;
    }
    ["trigger_description", "preferred_trigger"]
        .iter()
        .find_map(|k| {
            lookup(dict, k)
                .and_then(|v| v.get::<String>())
                .filter(|s| !s.is_empty())
        })
        .map(|s| vec![s])
        .unwrap_or_default()
}

/// privacy.app's `globalShortcuts.shortcuts`: id, description and accelerators of each entry, and every key of
/// the dictionary as JSON (`details`).
pub(super) fn to_json(list: &[Shortcut]) -> Value {
    json!(list
        .iter()
        .map(|s| {
            let description = lookup(&s.dict, "description").and_then(|v| v.get::<String>()).unwrap_or_default();
            json!({ "id": s.id, "description": description, "accels": accels(&s.dict), "details": variant_json(&s.dict) })
        })
        .collect::<Vec<_>>())
}

/// The entry's dictionary with new accelerators: `shortcuts` (and `preferred_trigger` when the entry has one).
pub(super) fn with_accels(dict: &glib::Variant, new: &[String]) -> glib::Variant {
    let d = glib::VariantDict::new(Some(dict));
    if d.contains("preferred_trigger") {
        d.insert_value(
            "preferred_trigger",
            &new.first().cloned().unwrap_or_default().to_variant(),
        );
    }
    if d.contains("shortcuts") || !d.contains("preferred_trigger") {
        d.insert_value("shortcuts", &new.to_vec().to_variant());
    }
    d.end()
}

/// The `RebindShortcuts` arguments: every registered entry, those named in `changes` with their new accelerators,
/// all of them without any when `clear` ("Alles verwijderen").
pub(super) fn rebind_args(
    app: &str,
    current: &[Shortcut],
    changes: &[(String, Vec<String>)],
    clear: bool,
) -> glib::Variant {
    let ty = glib::VariantTy::new("(sa{sv})").expect("a valid type");
    let items: Vec<glib::Variant> = current
        .iter()
        .map(|s| {
            let dict = if clear {
                with_accels(&s.dict, &[])
            } else {
                match changes.iter().find(|(id, _)| *id == s.id) {
                    Some((_, accels)) => with_accels(&s.dict, accels),
                    None => s.dict.clone(),
                }
            };
            glib::Variant::tuple_from_iter([s.id.to_variant(), dict])
        })
        .collect();
    glib::Variant::tuple_from_iter([
        app.to_variant(),
        glib::Variant::array_from_iter_with_type(ty, items),
    ])
}

/// Send the rebind to the portal.
pub(super) fn rebind(
    bus: &gio::DBusConnection,
    name: &str,
    args: &glib::Variant,
) -> Result<(), String> {
    bus.call_sync(
        Some(name),
        REBIND_PATH,
        REBIND_IFACE,
        "RebindShortcuts",
        Some(args),
        None,
        gio::DBusCallFlags::NONE,
        10_000,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// A GTK accelerator name as the API takes it (`<Control><Alt>t`): printable, no control characters, short.
pub(super) fn accel_ok(accel: &str) -> bool {
    !accel.is_empty() && accel.len() <= 64 && !accel.chars().any(char::is_control)
}
