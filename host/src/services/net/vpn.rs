//! VPN specifics of `net.*` (notes/claude/settings-parity.md §5.4): the installed NetworkManager VPN plugins
//! (`net.vpn.plugins`, as GNOME's Add VPN list: libnm's `nm_vpn_plugin_info_list_load` of the `.name` files) and
//! "Importeren uit bestand…" (`net.vpn.import`: libnm's offline import followed by an interactive AddConnection2 for
//! an OpenVPN `.ovpn` and NetworkManager's WireGuard importer for a `.conf`, as GNOME's Import from file…).
//! WireGuard is NetworkManager's own connection type: always offered.
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where NetworkManager's VPN plugins describe themselves (libnm: the lib directory first, then /etc).
pub(crate) const DIRS: &[&str] = &["/usr/lib/NetworkManager/VPN", "/etc/NetworkManager/VPN"];

/// The installed libnm plugins with an offline importer (NetworkManager-openvpn and -vpnc implement it).
const IMPORTERS: &[&str] = &["openvpn", "vpnc"];

/// One `.name` file: `[VPN Connection] name, service, program` and the editor groups; None without name and
/// service.
pub(crate) fn parse_name_file(text: &str) -> Option<Value> {
    let mut group = String::new();
    let (mut name, mut service, mut program, mut gtk) = (None, None, None, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g.to_owned();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match (group.as_str(), k) {
            ("VPN Connection", "name") => name = Some(v.to_owned()),
            ("VPN Connection", "service") => service = Some(v.to_owned()),
            ("VPN Connection", "program") => program = Some(v.to_owned()),
            ("GNOME", "properties") | ("libnm", "plugin") => gtk = true,
            _ => {}
        }
    }
    let (name, service) = (
        name.filter(|n| !n.is_empty())?,
        service.filter(|s| !s.is_empty())?,
    );
    let import = IMPORTERS.contains(&name.as_str());
    Some(json!({
        "name": name, "service": service, "program": program.unwrap_or_default(),
        "editor": gtk, "import": import,
    }))
}

/// net.vpn.plugins: every installed plugin (a service once: the first directory wins), and WireGuard.
pub(crate) fn plugins(dirs: &[&Path]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "name"))
            .collect();
        files.sort();
        for f in files {
            let Some(p) = std::fs::read_to_string(&f)
                .ok()
                .filter(|t| t.len() <= 64 * 1024)
                .and_then(|t| parse_name_file(&t))
            else {
                continue;
            };
            if !out.iter().any(|o| o["service"] == p["service"]) {
                out.push(p);
            }
        }
    }
    json!({ "plugins": out, "wireguard": true })
}

/// The kind of an import file: `wireguard` for a WireGuard `[Interface]` file, `openvpn` for an OpenVPN
/// configuration, `vpnc` for a Cisco `.pcf`; None for anything else.
pub(crate) fn detect(name: &str, text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower.contains("[interface]") && lower.contains("privatekey") {
        return Some("wireguard");
    }
    if ext == "pcf" || (lower.contains("[main]") && lower.contains("host=")) {
        return Some("vpnc");
    }
    let openvpn = lower.lines().map(str::trim).any(|l| {
        l == "client"
            || l.starts_with("remote ")
            || l.starts_with("<ca>")
            || l.starts_with("dev tun")
            || l.starts_with("dev tap")
    });
    if ext == "ovpn" || openvpn {
        return Some("openvpn");
    }
    None
}

/// NetworkManager names a WireGuard import after its file (wg0.conf → interface wg0): the stem must be an interface
/// name.
pub(crate) fn wireguard_name_ok(path: &str) -> bool {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(super::iface_ok)
}

#[cfg(test)]
/// The uuid in nmcli's "Connection 'name' (uuid) successfully added."
pub(crate) fn added_uuid(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let (_, rest) = l.rsplit_once('(')?;
        let (uuid, _) = rest.split_once(')')?;
        super::uuid_ok(uuid).then(|| uuid.to_ascii_lowercase())
    })
}

#[cfg(test)]
/// The first line nmcli wrote about a failure, without its "Error: " (at most 300 characters).
pub(crate) fn nmcli_error(err: &str, out: &str) -> String {
    let line = err
        .lines()
        .chain(out.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    line.strip_prefix("Error: ")
        .unwrap_or(line)
        .chars()
        .take(300)
        .collect()
}

fn bad_import(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "net.badImport", NO_PARAMS)
}

/// The checked import: (file, kind).
pub(crate) fn import_args(args: &Value, lang: Lang) -> Result<(String, &'static str), ApiError> {
    let path = args["path"]
        .as_str()
        .filter(|p| p.starts_with('/') && p.len() <= 4096 && !p.contains(['\0', '\n']))
        .ok_or_else(|| ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS))?;
    let meta = std::fs::metadata(path).map_err(|_| bad_import(lang))?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err(bad_import(lang));
    }
    let text = std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|_| bad_import(lang))?;
    let kind = match args["type"].as_str() {
        Some("openvpn") => "openvpn",
        Some("wireguard") => "wireguard",
        Some("vpnc") => "vpnc",
        Some(_) => return Err(ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS)),
        None => detect(path, &text).ok_or_else(|| bad_import(lang))?,
    };
    if kind == "wireguard" && !wireguard_name_ok(path) {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "net.badImportName",
            NO_PARAMS,
        ));
    }
    Ok((path.to_owned(), kind))
}

/// Whether the plugin of a kind is installed (WireGuard is NetworkManager's own).
pub(crate) fn kind_installed(kind: &str, plugins: &Value) -> bool {
    kind == "wireguard"
        || plugins["plugins"]
            .as_array()
            .is_some_and(|a| a.iter().any(|p| p["name"] == kind))
}

/// Find the selected installed plugin's own description; a client never supplies a library path.
fn plugin_file(kind: &str) -> Option<PathBuf> {
    for dir in DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "name"))
            .collect();
        paths.sort();
        for path in paths {
            if std::fs::read_to_string(&path)
                .ok()
                .filter(|s| s.len() <= 64 * 1024)
                .and_then(|s| parse_name_file(&s))
                .is_some_and(|p| p["name"] == kind)
            {
                return Some(path);
            }
        }
    }
    None
}

/// Save the fully typed offline import with NetworkManager's own interactive authorization.
pub(super) fn save_import(
    bus: &gio::DBusConnection,
    groups: &super::settings_map::Groups,
    kind: &str,
    lang: Lang,
) -> ApiResult {
    let uuid = groups
        .get("connection")
        .and_then(|g| g.get("uuid"))
        .and_then(|v| v.get::<String>())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_import(lang))?;
    super::connections::add(bus, groups, lang)?;
    Ok(json!({"uuid": uuid, "type": kind}))
}

/// Parsing is offline; only AddConnection2 contacts the daemon, with ALLOW_INTERACTIVE_AUTHORIZATION.
pub(crate) fn import(path: &str, kind: &str, lang: Lang) -> ApiResult {
    let dirs: Vec<&Path> = DIRS.iter().map(Path::new).collect();
    if !kind_installed(kind, &plugins(&dirs)) {
        return Err(ApiError::tr("unsupported", lang, "net.noPlugin", NO_PARAMS));
    }
    let file = plugin_file(kind);
    let groups = super::vpn_import::parse(path, kind, file.as_deref(), lang)?;
    let bus = super::system_bus(lang)?;
    save_import(&bus, &groups, kind, lang)
}
