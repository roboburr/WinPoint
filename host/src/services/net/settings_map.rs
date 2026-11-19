//! NetworkManager's connection settings (`a{sa{sv}}`) and their JSON (notes/claude/settings-parity.md §5.4).
//!
//! `net.connection.get` sends every key as JSON (`variant_json`: byte arrays such as the SSID as arrays of numbers,
//! `address-data` and `route-data` as arrays of objects). `net.connection.update` never rebuilds a connection: it
//! merges the page's changes into NM's own `GetSettings` answer, so every key the page does not name, also one this
//! table does not know, goes back to NM exactly as NM sent it (amendment of the plan's risk: "never rebuild a whole
//! connection"). A changed value gets its D-Bus type from [`kind_of`], or else from the value NM sent for that key;
//! a key with neither is refused.
use crate::services::system::common::variant_json;
use glib::Variant;
use glib::variant::ToVariant;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The settings of one connection: setting name → key → value.
pub(crate) type Groups = BTreeMap<String, BTreeMap<String, Variant>>;

/// What a change may not do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MapError {
    /// The changes are not `{group: {key: value | null} | null}`, or name a group or key badly.
    BadArgs,
    /// This key's value does not fit its type (or the key has no known type).
    BadValue { group: String, key: String },
}

/// The D-Bus shapes of the keys WinPoint's pages write.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Bool,
    I32,
    U32,
    I64,
    U64,
    Str,
    Strs,
    U32s,
    /// `ay`: an array of 0..=255, or a text (its UTF-8 bytes), e.g. the SSID.
    Bytes,
    /// `ay` of 6 bytes: an array, or the text `aa:bb:cc:dd:ee:ff`.
    Mac,
    /// `ay` of a certificate: an array, or a path (NM's `file://<path>` scheme with its trailing NUL).
    Cert,
    /// `aay`.
    ByteArrays,
    /// `aa{sv}` of `{address: s, prefix: u}` (ipv4/ipv6 `address-data`).
    AddressData,
    /// `aa{sv}` of `{dest: s, prefix: u, next-hop: s, metric: u, table: u}` (`route-data`).
    RouteData,
    /// `a{ss}` (`vpn.data`, `vpn.secrets`).
    StrDict,
    /// `aa{sv}` of WireGuard peers: `{public-key: s, endpoint: s, preshared-key: s, preshared-key-flags: u,
    /// allowed-ips: as, persistent-keepalive: u}` (`wireguard.peers`).
    Peers,
}

/// The type of a key WinPoint knows (nm-settings-dbus(5), NetworkManager 1.54).
pub(crate) fn kind_of(group: &str, key: &str) -> Option<Kind> {
    use Kind::*;
    let ip = matches!(group, "ipv4" | "ipv6");
    Some(match (group, key) {
        ("connection", "id" | "uuid" | "type" | "interface-name" | "zone" | "stable-id") => Str,
        ("connection", "autoconnect") => Bool,
        ("connection", "autoconnect-priority" | "metered" | "dns-over-tls" | "mdns" | "llmnr") => {
            I32
        }
        ("connection", "permissions") => Strs,
        ("802-11-wireless", "ssid") => Bytes,
        ("802-11-wireless", "mode" | "band" | "assigned-mac-address") => Str,
        ("802-11-wireless", "hidden") => Bool,
        ("802-11-wireless", "bssid" | "mac-address" | "cloned-mac-address") => Mac,
        ("802-11-wireless", "channel" | "mtu" | "powersave") => U32,
        ("802-11-wireless-security", "key-mgmt" | "auth-alg" | "leap-username") => Str,
        ("802-11-wireless-security", "psk" | "leap-password") => Str,
        ("802-11-wireless-security", "wep-key0" | "wep-key1" | "wep-key2" | "wep-key3") => Str,
        (
            "802-11-wireless-security",
            "psk-flags"
            | "wep-key-flags"
            | "wep-key-type"
            | "wep-tx-keyidx"
            | "leap-password-flags",
        ) => U32,
        ("802-11-wireless-security", "pmf" | "fils") => I32,
        ("802-11-wireless-security", "proto" | "pairwise" | "group") => Strs,
        ("802-1x", "eap" | "altsubject-matches") => Strs,
        (
            "802-1x",
            "identity"
            | "anonymous-identity"
            | "password"
            | "phase1-peapver"
            | "phase1-peaplabel"
            | "phase1-fast-provisioning"
            | "phase2-auth"
            | "phase2-autheap"
            | "domain-suffix-match"
            | "domain-match"
            | "subject-match"
            | "pac-file"
            | "private-key-password"
            | "ca-cert-password"
            | "client-cert-password"
            | "phase2-private-key-password"
            | "pin"
            | "ca-path",
        ) => Str,
        (
            "802-1x",
            "password-flags"
            | "private-key-password-flags"
            | "ca-cert-password-flags"
            | "client-cert-password-flags"
            | "phase2-private-key-password-flags"
            | "pin-flags",
        ) => U32,
        (
            "802-1x",
            "ca-cert" | "client-cert" | "private-key" | "phase2-ca-cert" | "phase2-client-cert"
            | "phase2-private-key",
        ) => Cert,
        ("802-1x", "system-ca-certs" | "optional") => Bool,
        ("802-1x", "auth-timeout") => I32,
        (_, "method" | "gateway" | "dhcp-hostname") if ip => Str,
        (_, "address-data") if ip => AddressData,
        (_, "route-data") if ip => RouteData,
        (_, "dns-data" | "dns-search") if ip => Strs,
        (
            _,
            "ignore-auto-dns" | "ignore-auto-routes" | "never-default" | "may-fail"
            | "dhcp-send-hostname",
        ) if ip => Bool,
        (_, "route-metric") if ip => I64,
        (_, "dns-priority" | "required-timeout") if ip => I32,
        ("ipv6", "addr-gen-mode" | "ip6-privacy") => I32,
        ("802-3-ethernet", "mac-address" | "cloned-mac-address") => Mac,
        ("802-3-ethernet", "assigned-mac-address" | "duplex" | "port") => Str,
        ("802-3-ethernet", "mtu" | "speed" | "wake-on-lan") => U32,
        ("802-3-ethernet", "auto-negotiate") => Bool,
        ("bluetooth", "bdaddr") => Mac,
        ("bluetooth", "type") => Str,
        ("vpn", "service-type" | "user-name") => Str,
        ("vpn", "data" | "secrets") => StrDict,
        ("vpn", "persistent") => Bool,
        ("vpn", "timeout") => U32,
        // WireGuard (GNOME's WireGuard tab, ce-page-wireguard.c)
        ("wireguard", "private-key") => Str,
        ("wireguard", "private-key-flags" | "listen-port" | "fwmark" | "mtu") => U32,
        ("wireguard", "peer-routes") => Bool,
        ("wireguard", "ip4-auto-default-route" | "ip6-auto-default-route") => I32,
        ("wireguard", "peers") => Peers,
        // mobile broadband (GNOME's Mobile Network panel: data, roaming, APN)
        (
            "gsm",
            "apn"
            | "username"
            | "password"
            | "pin"
            | "sim-id"
            | "sim-operator-id"
            | "device-id"
            | "network-id"
            | "number"
            | "initial-eps-bearer-apn"
            | "initial-eps-bearer-username"
            | "initial-eps-bearer-password",
        ) => Str,
        (
            "gsm",
            "home-only"
            | "auto-config"
            | "initial-eps-bearer-configure"
            | "initial-eps-bearer-noauth"
            | "initial-eps-bearer-refuse-pap"
            | "initial-eps-bearer-refuse-chap"
            | "initial-eps-bearer-refuse-eap"
            | "initial-eps-bearer-refuse-mschap"
            | "initial-eps-bearer-refuse-mschapv2",
        ) => Bool,
        ("gsm", "password-flags" | "pin-flags" | "mtu" | "initial-eps-bearer-password-flags") => {
            U32
        }
        (
            "ppp",
            "refuse-eap" | "refuse-pap" | "refuse-chap" | "refuse-mschap" | "refuse-mschapv2"
            | "noauth" | "require-mppe" | "require-mppe-128" | "mppe-stateful" | "nobsdcomp"
            | "nodeflate" | "no-vj-comp",
        ) => Bool,
        ("ppp", "lcp-echo-failure" | "lcp-echo-interval" | "mtu" | "mru") => U32,
        _ => return None,
    })
}

/// The kind of a value NM sent (the fallback for a key [`kind_of`] does not list).
pub(crate) fn kind_from_type(t: &str) -> Option<Kind> {
    use Kind::*;
    Some(match t {
        "b" => Bool,
        "i" => I32,
        "u" => U32,
        "x" => I64,
        "t" => U64,
        "s" => Str,
        "as" => Strs,
        "au" => U32s,
        "ay" => Bytes,
        "aay" => ByteArrays,
        "a{ss}" => StrDict,
        _ => return None,
    })
}

/// The keys that hold a password, key or PIN (amendment 53: never logged; their presence makes the update carry the
/// connection's other secrets too, [`has_secrets`]).
pub(crate) fn is_secret(group: &str, key: &str) -> bool {
    matches!(
        (group, key),
        (
            "802-11-wireless-security",
            "psk" | "wep-key0" | "wep-key1" | "wep-key2" | "wep-key3" | "leap-password"
        ) | (
            "802-1x",
            "password"
                | "password-raw"
                | "pin"
                | "private-key-password"
                | "ca-cert-password"
                | "client-cert-password"
                | "phase2-private-key-password"
                | "phase2-ca-cert-password"
                | "phase2-client-cert-password"
        ) | ("vpn", "secrets")
            | ("wireguard", "private-key")
            | ("gsm", "password" | "pin" | "initial-eps-bearer-password")
            | ("cdma", "password")
            | ("pppoe", "password")
    )
}

/// The setting groups whose secrets NM keeps apart (GetSecrets asks for them by name).
pub(crate) const SECRET_GROUPS: &[&str] = &[
    "802-11-wireless-security",
    "802-1x",
    "vpn",
    "wireguard",
    "gsm",
    "cdma",
    "pppoe",
];

/// Whether a change names a secret key (also a WireGuard peer with a pre-shared key).
pub(crate) fn has_secrets(changes: &Value) -> bool {
    let peer_secret = |v: &Value| {
        v.as_array().is_some_and(|peers| {
            peers
                .iter()
                .any(|p| p["preshared-key"].as_str().is_some_and(|k| !k.is_empty()))
        })
    };
    changes.as_object().is_some_and(|groups| {
        groups.iter().any(|(g, keys)| {
            keys.as_object().is_some_and(|k| {
                k.iter().any(|(key, v)| {
                    !v.is_null()
                        && (is_secret(g, key)
                            || (g == "wireguard" && key == "peers" && peer_secret(v)))
                })
            })
        })
    })
}

/// The deprecated D-Bus keys NM prefers when they are present (libnm-core `_nm_setting_use_legacy_property`): the
/// new key that replaces each. A merged group that has the new key loses the old one, or NM would ignore the change.
const LEGACY: &[(&str, &str, &str)] = &[
    ("ipv4", "address-data", "addresses"),
    ("ipv4", "route-data", "routes"),
    ("ipv4", "dns-data", "dns"),
    ("ipv6", "address-data", "addresses"),
    ("ipv6", "route-data", "routes"),
    ("ipv6", "dns-data", "dns"),
    (
        "802-11-wireless",
        "assigned-mac-address",
        "cloned-mac-address",
    ),
    (
        "802-3-ethernet",
        "assigned-mac-address",
        "cloned-mac-address",
    ),
];

fn name_ok(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn text_ok(s: &str) -> bool {
    s.len() <= 4096 && !s.contains('\0')
}

/// a{sa{sv}} → groups.
pub(crate) fn groups_of(v: &Variant) -> Groups {
    let mut out = Groups::new();
    if !v.type_().as_str().starts_with("a{sa{sv}}") {
        return out;
    }
    for i in 0..v.n_children() {
        let e = v.child_value(i);
        let Some(name) = e.child_value(0).str().map(str::to_owned) else {
            continue;
        };
        let dict = e.child_value(1);
        let mut keys = BTreeMap::new();
        for j in 0..dict.n_children() {
            let kv = dict.child_value(j);
            if let (Some(k), Some(value)) =
                (kv.child_value(0).str(), kv.child_value(1).as_variant())
            {
                keys.insert(k.to_owned(), value);
            }
        }
        out.insert(name, keys);
    }
    out
}

/// The groups as JSON (byte arrays as numbers; nothing left out).
pub(crate) fn to_json(groups: &Groups) -> Value {
    let mut out = Map::new();
    for (g, keys) in groups {
        let mut m = Map::new();
        for (k, v) in keys {
            m.insert(k.clone(), variant_json(v));
        }
        out.insert(g.clone(), Value::Object(m));
    }
    Value::Object(out)
}

/// A MAC address as NM's six bytes.
pub(crate) fn parse_mac(text: &str) -> Option<Vec<u8>> {
    let parts: Vec<&str> = text.trim().split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    parts
        .iter()
        .map(|p| {
            (p.len() == 2)
                .then(|| u8::from_str_radix(p, 16).ok())
                .flatten()
        })
        .collect()
}

/// Six bytes as `AA:BB:CC:DD:EE:FF` (the pages format NM's bytes themselves; the tests compare with it).
#[cfg(test)]
pub(crate) fn format_mac(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A certificate path as NM's `file://<path>` blob (with its NUL), or the path it holds.
pub(crate) fn cert_blob(path: &str) -> Vec<u8> {
    let mut v = format!("file://{path}").into_bytes();
    v.push(0);
    v
}

/// The path of a `file://` certificate blob; None for an inline certificate.
#[cfg(test)]
pub(crate) fn cert_path(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.strip_suffix(&[0]).unwrap_or(bytes)).ok()?;
    text.strip_prefix("file://").map(str::to_owned)
}

fn bytes_of(v: &Value, max: usize) -> Option<Vec<u8>> {
    match v {
        Value::String(s) if s.len() <= max => Some(s.as_bytes().to_vec()),
        Value::Array(a) if a.len() <= max => a
            .iter()
            .map(|b| b.as_u64().filter(|b| *b <= 255).map(|b| b as u8))
            .collect(),
        _ => None,
    }
}

fn strs_of(v: &Value) -> Option<Vec<String>> {
    let a = v.as_array().filter(|a| a.len() <= 256)?;
    a.iter()
        .map(|s| s.as_str().filter(|s| text_ok(s)).map(str::to_owned))
        .collect()
}

fn record(v: &Value, fields: &[(&str, Kind)]) -> Option<BTreeMap<String, Variant>> {
    let o = v.as_object()?;
    let mut out = BTreeMap::new();
    for (k, x) in o {
        let kind = fields
            .iter()
            .find(|(name, _)| name == k)
            .map(|(_, kind)| *kind)?;
        out.insert(k.clone(), value_variant(kind, x)?);
    }
    Some(out)
}

/// One JSON value as the variant of `kind`; None when it does not fit.
pub(crate) fn value_variant(kind: Kind, v: &Value) -> Option<Variant> {
    use Kind::*;
    Some(match kind {
        Bool => v.as_bool()?.to_variant(),
        I32 => i32::try_from(v.as_i64()?).ok()?.to_variant(),
        U32 => u32::try_from(v.as_u64()?).ok()?.to_variant(),
        I64 => v.as_i64()?.to_variant(),
        U64 => v.as_u64()?.to_variant(),
        Str => v.as_str().filter(|s| text_ok(s))?.to_variant(),
        Strs => strs_of(v)?.to_variant(),
        U32s => {
            let a = v.as_array().filter(|a| a.len() <= 256)?;
            a.iter()
                .map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
                .collect::<Option<Vec<u32>>>()?
                .to_variant()
        }
        Bytes => bytes_of(v, 4096)?.to_variant(),
        Mac => match v {
            Value::String(s) => parse_mac(s)?.to_variant(),
            _ => bytes_of(v, 6).filter(|b| b.len() == 6)?.to_variant(),
        },
        Cert => match v {
            Value::String(s) if text_ok(s) && s.starts_with('/') => cert_blob(s).to_variant(),
            Value::Array(_) => bytes_of(v, 65536)?.to_variant(),
            _ => return None,
        },
        ByteArrays => {
            let a = v.as_array().filter(|a| a.len() <= 64)?;
            a.iter()
                .map(|b| bytes_of(b, 64))
                .collect::<Option<Vec<Vec<u8>>>>()?
                .to_variant()
        }
        AddressData => {
            let a = v.as_array().filter(|a| a.len() <= 64)?;
            let rows = a
                .iter()
                .map(|r| {
                    record(r, &[("address", Str), ("prefix", U32)])
                        .filter(|m| m.contains_key("address") && m.contains_key("prefix"))
                })
                .collect::<Option<Vec<_>>>()?;
            rows.to_variant()
        }
        RouteData => {
            let a = v.as_array().filter(|a| a.len() <= 256)?;
            let fields = [
                ("dest", Str),
                ("prefix", U32),
                ("next-hop", Str),
                ("metric", U32),
                ("table", U32),
            ];
            let rows = a
                .iter()
                .map(|r| {
                    record(r, &fields)
                        .filter(|m| m.contains_key("dest") && m.contains_key("prefix"))
                })
                .collect::<Option<Vec<_>>>()?;
            rows.to_variant()
        }
        StrDict => {
            let o = v.as_object().filter(|o| o.len() <= 256)?;
            let mut m = BTreeMap::new();
            for (k, x) in o {
                if !text_ok(k) {
                    return None;
                }
                m.insert(k.clone(), x.as_str().filter(|s| text_ok(s))?.to_owned());
            }
            m.to_variant()
        }
        Peers => {
            let a = v.as_array().filter(|a| a.len() <= 256)?;
            let fields = [
                ("public-key", Str),
                ("endpoint", Str),
                ("preshared-key", Str),
                ("preshared-key-flags", U32),
                ("allowed-ips", Strs),
                ("persistent-keepalive", U32),
            ];
            let rows = a
                .iter()
                .map(|r| {
                    record(r, &fields).filter(|m| {
                        m.get("public-key")
                            .and_then(|k| k.str().map(|s| !s.is_empty()))
                            .unwrap_or(false)
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            rows.to_variant()
        }
    })
}

/// `vpn.secrets` of a change over the stored ones: a text sets one secret, null removes it, the others stay (the
/// page sends only what it changed; [`add_secrets`] brought the stored ones in).
fn overlay_secrets(old: Option<&Variant>, change: &Value) -> Option<Variant> {
    let mut m: BTreeMap<String, String> = old.and_then(|o| o.get()).unwrap_or_default();
    for (k, x) in change.as_object().filter(|o| o.len() <= 256)? {
        if !text_ok(k) {
            return None;
        }
        match x {
            Value::Null => {
                m.remove(k);
            }
            Value::String(s) if text_ok(s) => {
                m.insert(k.clone(), s.clone());
            }
            _ => return None,
        }
    }
    Some(m.to_variant())
}

/// The pre-shared key (and its flags) of every WireGuard peer of `old`, by public key.
fn peer_secrets(old: &Variant) -> BTreeMap<String, Vec<(String, Variant)>> {
    let mut out = BTreeMap::new();
    for i in 0..old.n_children() {
        let peer: BTreeMap<String, Variant> = old.child_value(i).get().unwrap_or_default();
        let Some(key) = peer
            .get("public-key")
            .and_then(|k| k.str().map(str::to_owned))
        else {
            continue;
        };
        let kept: Vec<(String, Variant)> = peer
            .into_iter()
            .filter(|(k, _)| k == "preshared-key" || k == "preshared-key-flags")
            .collect();
        if !kept.is_empty() {
            out.insert(key, kept);
        }
    }
    out
}

/// New WireGuard peers that keep the pre-shared key their public key had (the page sends a peer's key only when it
/// was typed again).
fn keep_peer_secrets(new: Variant, old: Option<&Variant>) -> Variant {
    let Some(old) = old else { return new };
    let secrets = peer_secrets(old);
    let mut rows = Vec::new();
    for i in 0..new.n_children() {
        let mut peer: BTreeMap<String, Variant> = new.child_value(i).get().unwrap_or_default();
        let key = peer
            .get("public-key")
            .and_then(|k| k.str().map(str::to_owned));
        if let Some(kept) = key.and_then(|k| secrets.get(&k))
            && !peer.contains_key("preshared-key")
        {
            for (k, v) in kept {
                peer.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        rows.push(peer);
    }
    rows.to_variant()
}

/// Merge `changes` (`{group: {key: value | null} | null}`) into `base`: a value replaces the key (typed by
/// [`kind_of`], else by the value NM sent), null removes the key (NM's default again), a null group removes the
/// group; every key the changes do not name stays as it was. `connection` and its id, uuid and type stay.
pub(crate) fn merge(base: &mut Groups, changes: &Value) -> Result<(), MapError> {
    let groups = changes
        .as_object()
        .filter(|g| g.len() <= 64)
        .ok_or(MapError::BadArgs)?;
    for (g, keys) in groups {
        if !name_ok(g, 48) {
            return Err(MapError::BadArgs);
        }
        if keys.is_null() {
            if g == "connection" {
                return Err(MapError::BadArgs);
            }
            base.remove(g);
            continue;
        }
        let keys = keys
            .as_object()
            .filter(|k| k.len() <= 200)
            .ok_or(MapError::BadArgs)?;
        let group = base.entry(g.clone()).or_default();
        for (k, v) in keys {
            if !name_ok(k, 64) {
                return Err(MapError::BadArgs);
            }
            let bad = || MapError::BadValue {
                group: g.clone(),
                key: k.clone(),
            };
            if v.is_null() {
                if g == "connection" && matches!(k.as_str(), "id" | "uuid" | "type") {
                    return Err(bad());
                }
                group.remove(k);
                continue;
            }
            if (g.as_str(), k.as_str()) == ("vpn", "secrets") {
                let value = overlay_secrets(group.get(k), v).ok_or_else(bad)?;
                group.insert(k.clone(), value);
                continue;
            }
            let kind = kind_of(g, k)
                .or_else(|| {
                    group
                        .get(k)
                        .and_then(|old| kind_from_type(old.type_().as_str()))
                })
                .ok_or_else(bad)?;
            let mut value = value_variant(kind, v).ok_or_else(bad)?;
            if kind == Kind::Peers {
                value = keep_peer_secrets(value, group.get(k));
            }
            group.insert(k.clone(), value);
        }
    }
    for (g, new, old) in LEGACY {
        if let Some(group) = base.get_mut(*g)
            && group.contains_key(*new)
        {
            group.remove(*old);
        }
    }
    let connection = base.get("connection").ok_or(MapError::BadArgs)?;
    for k in ["id", "type"] {
        if !connection
            .get(k)
            .is_some_and(|v| v.str().is_some_and(|s| !s.is_empty()))
        {
            return Err(MapError::BadValue {
                group: "connection".into(),
                key: k.into(),
            });
        }
    }
    Ok(())
}

/// A connection from nothing but `settings` (`net.connection.add`): every key typed by [`kind_of`].
pub(crate) fn build(settings: &Value) -> Result<Groups, MapError> {
    if settings
        .as_object()
        .is_some_and(|g| g.values().any(Value::is_null))
    {
        return Err(MapError::BadArgs);
    }
    let mut groups = Groups::new();
    merge(&mut groups, settings)?;
    Ok(groups)
}

/// The groups as NM's `a{sa{sv}}` (the tests' round trip; the D-Bus calls put the dictionary itself in their tuples).
#[cfg(test)]
pub(crate) fn to_variant(groups: &Groups) -> Variant {
    groups.to_variant()
}

/// Merge NM's secrets (a `GetSecrets` answer) into `base`, without replacing what is there: a key NM left out of
/// GetSettings is added, the entries of `vpn.secrets` join the dictionary, and each WireGuard peer gets the
/// pre-shared key of its public key.
pub(crate) fn add_secrets(base: &mut Groups, secrets: &Groups) {
    for (g, keys) in secrets {
        let group = base.entry(g.clone()).or_default();
        for (k, v) in keys {
            let merged = match group.get(k) {
                None => v.clone(),
                Some(old) if old.type_().as_str() == "a{ss}" && v.type_().as_str() == "a{ss}" => {
                    let mut m: BTreeMap<String, String> = old.get().unwrap_or_default();
                    for (sk, sv) in v.get::<BTreeMap<String, String>>().unwrap_or_default() {
                        m.entry(sk).or_insert(sv);
                    }
                    m.to_variant()
                }
                Some(old) if g == "wireguard" && k == "peers" => {
                    keep_peer_secrets(old.clone(), Some(v))
                }
                Some(_) => continue,
            };
            group.insert(k.clone(), merged);
        }
    }
}

/// The text of a JSON SSID (bytes or text), lossy.
pub(crate) fn ssid_text(v: &Value) -> String {
    match v {
        Value::Array(a) => String::from_utf8_lossy(
            &a.iter()
                .filter_map(|b| b.as_u64().map(|b| b as u8))
                .collect::<Vec<_>>(),
        )
        .into_owned(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// The QR text of a Wi-Fi network (gnome-control-center's `get_qr_string`, the ZXing `WIFI:` format): `S` the
/// network name, `T` WPA, SAE or WEP (`nopass` without a password), `P` the password, `H:true` for a hidden
/// network; `\ ; , : "` escaped with a backslash.
pub(crate) fn wifi_qr(ssid: &str, security: &str, password: Option<&str>, hidden: bool) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
    let mut s = format!("WIFI:S:{};T:", esc(ssid));
    match password.filter(|p| !p.is_empty()) {
        Some(p) => {
            s.push_str(security);
            s.push_str(";P:");
            s.push_str(&esc(p));
        }
        None => s.push_str("nopass"),
    }
    s.push(';');
    if hidden {
        s.push_str("H:true;");
    }
    s.push(';');
    s
}

/// The `T:` value of a key management (gnome-control-center `get_wifi_security_type`): WPA for WPA-PSK, SAE for WPA3,
/// WEP for a WEP key, `nopass` for an open network.
pub(crate) fn qr_security(key_mgmt: &str) -> &'static str {
    match key_mgmt {
        "wpa-psk" => "WPA",
        "sae" => "SAE",
        "none" | "ieee8021x" => "WEP",
        _ => "nopass",
    }
}

/// The 802-11-wireless-security (and 802-1x) groups of a security kind, before the page's own fields:
/// `open`, `owe`, `wep` (key), `wep-passphrase`, `leap`, `dynamic-wep`, `wpa-psk`, `sae`, `wpa-eap`
/// (gnome-control-center's security list; libnma `NMAWs*`).
pub(crate) fn security_base(kind: &str) -> Option<Value> {
    use serde_json::json;
    Some(match kind {
        "open" => json!({ "802-11-wireless-security": null, "802-1x": null }),
        "owe" => json!({ "802-11-wireless-security": { "key-mgmt": "owe" }, "802-1x": null }),
        "wep" => {
            json!({ "802-11-wireless-security": { "key-mgmt": "none", "auth-alg": "open", "wep-key-type": 1 }, "802-1x": null })
        }
        "wep-passphrase" => {
            json!({ "802-11-wireless-security": { "key-mgmt": "none", "auth-alg": "open", "wep-key-type": 2 }, "802-1x": null })
        }
        "leap" => {
            json!({ "802-11-wireless-security": { "key-mgmt": "ieee8021x", "auth-alg": "leap" }, "802-1x": null })
        }
        "dynamic-wep" => {
            json!({ "802-11-wireless-security": { "key-mgmt": "ieee8021x", "auth-alg": "open" } })
        }
        "wpa-psk" => {
            json!({ "802-11-wireless-security": { "key-mgmt": "wpa-psk" }, "802-1x": null })
        }
        "sae" => json!({ "802-11-wireless-security": { "key-mgmt": "sae" }, "802-1x": null }),
        "wpa-eap" => json!({ "802-11-wireless-security": { "key-mgmt": "wpa-eap" } }),
        _ => return None,
    })
}

/// Deep-merge JSON change objects (`b` over `a`).
pub(crate) fn overlay(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in y {
                match x.get_mut(k) {
                    Some(old) if old.is_object() && v.is_object() => overlay(old, v),
                    _ => {
                        x.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}
