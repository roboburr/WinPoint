//! Finding printers and their drivers, as GNOME's Add Printer dialog does (pp-new-printer-dialog.c, pp-host.c,
//! pp-utils.c) without system-config-printer's Python services (plan.gnomeFunctionsLeftOut):
//!
//! * the scheduler's backends: CupsPkHelper `DevicesGet` (local USB and the network backends cupsd runs; polkit
//!   devices-get) as `{uri, info, makeModel, deviceId, location, class}`;
//! * DNS-SD: Avahi's `_ipp._tcp` and `_ipps._tcp` services (their TXT `rp`, `ty`, `pdl`, `note`, `usb_MFG`,
//!   `usb_MDL`), driverless when `pdl` lists a raster or PDF format cupsd's `everywhere` model can drive;
//! * an address the user typed ("Een printer toevoegen met behulp van een IP-adres of hostnaam"): a print server's
//!   shared queues (CUPS-Get-Printers), an IPP printer (Get-Printer-Attributes on `/ipp/print`, driverless by its
//!   attributes), JetDirect (port 9100) and LPD (515) as GNOME probes them; `smb://` only as typed (amendment 41:
//!   libsmbclient is not loaded, so there is no SMB browsing);
//! * drivers: CUPS-Get-PPDs, ranked against the device's IEEE 1284 id and make-and-model (GNOME asks
//!   GetBestDrivers), with `everywhere` (cupsd builds the PPD from the printer itself, amendment 41) first for a
//!   driverless device.
use super::cups::{self, Attrs, Conn, Server, text, texts};
use super::ipp;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// One device of CupsPkHelper's DevicesGet answer (`a{ss}` with "device-uri:N" style keys, cph-cups.c).
pub(crate) fn devices_get(map: &serde_json::Map<String, Value>) -> Vec<Value> {
    let mut indices: Vec<u32> = map
        .keys()
        .filter_map(|k| k.strip_prefix("device-uri:"))
        .filter_map(|n| n.parse().ok())
        .collect();
    indices.sort_unstable();
    indices
        .into_iter()
        .filter_map(|n| {
            let get = |key: &str| {
                map.get(&format!("{key}:{n}"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let uri = get("device-uri");
            // a backend that found nothing reports its bare scheme ("ipp", "socket", "lpd", "smb")
            (uri.contains(':') && !uri.ends_with(':')).then(|| {
                let id = get("device-id");
                let (make, model) = ipp::device_id(&id);
                let mm = get("device-make-and-model");
                let make_model = if mm.is_empty() || mm.eq_ignore_ascii_case("unknown") {
                    format!("{make} {model}").trim().to_owned()
                } else {
                    mm
                };
                let class = get("device-class");
                json!({
                    "uri": uri,
                    "info": get("device-info"),
                    "makeModel": make_model,
                    "deviceId": id,
                    "location": get("device-location"),
                    "class": class,
                    "network": class == "network",
                    "source": "cups",
                    "driverless": uri.starts_with("ipp://") || uri.starts_with("ipps://") || uri.starts_with("ippusb://"),
                })
            })
        })
        .collect()
}

/// The TXT record of a DNS-SD printer as key → value.
pub(crate) fn txt_record(entries: &[Vec<u8>]) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for entry in entries {
        let text = String::from_utf8_lossy(entry);
        let (key, value) = text.split_once('=').unwrap_or((&text, ""));
        out.insert(key.to_ascii_lowercase(), json!(value));
    }
    out
}

/// A DNS-SD `_ipp._tcp` / `_ipps._tcp` service as a device.
pub(crate) fn dnssd_device(
    name: &str,
    kind: &str,
    host: &str,
    port: u16,
    txt: &serde_json::Map<String, Value>,
) -> Value {
    let get = |k: &str| txt.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let scheme = if kind.starts_with("_ipps") {
        "ipps"
    } else {
        "ipp"
    };
    let host = host.trim_end_matches('.');
    let rp = get("rp");
    let uri = format!("{scheme}://{host}:{port}/{}", rp.trim_start_matches('/'));
    let pdl = get("pdl");
    let make = get("usb_mfg");
    let model = get("usb_mdl");
    let ty = get("ty");
    let make_model = if !ty.is_empty() {
        ty
    } else {
        format!("{make} {model}").trim().to_owned()
    };
    let device_id = if make.is_empty() && model.is_empty() {
        String::new()
    } else {
        format!("MFG:{make};MDL:{model};")
    };
    // a CUPS server's shared queue carries printer-type; its driverless share works with `everywhere` too
    let remote_queue = txt.contains_key("printer-type");
    let driverless = remote_queue
        || pdl.split(',').any(|f| {
            matches!(
                f.trim(),
                "image/pwg-raster" | "image/urf" | "application/pdf"
            )
        });
    json!({
        "uri": uri,
        "info": name,
        "makeModel": make_model,
        "deviceId": device_id,
        "location": get("note"),
        "class": "network",
        "network": true,
        "source": "dnssd",
        "driverless": driverless,
        "remoteQueue": remote_queue,
    })
}

/// What the user typed in "Hostnaam of IP-adres": a URI as it is, or a host with an optional port.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    /// A full device URI (ipp, ipps, http, https, socket, lpd, smb, ...).
    Uri(String),
    /// A host (name, IPv4 or bracketless IPv6) and the port the user named.
    Host(String, Option<u16>),
}

/// The schemes a typed address may use (CUPS's network backends).
const SCHEMES: &[&str] = &["ipp", "ipps", "http", "https", "socket", "lpd", "smb"];

pub(crate) fn target(address: &str) -> Option<Target> {
    let a = address.trim();
    if a.is_empty() || a.len() > 1024 || a.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    if let Some((scheme, rest)) = a.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !SCHEMES.contains(&scheme.as_str()) || rest.is_empty() {
            return None;
        }
        return Some(Target::Uri(format!("{scheme}://{rest}")));
    }
    // "host", "host:port", "[v6]:port", "v6"
    let (host, port) = if let Some(rest) = a.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        (
            h.to_owned(),
            tail.strip_prefix(':')
                .map(|p| p.parse().ok())
                .unwrap_or(None),
        )
    } else if a.matches(':').count() == 1 {
        let (h, p) = a.split_once(':')?;
        (h.to_owned(), Some(p.parse().ok()?))
    } else {
        (a.to_owned(), None)
    };
    let ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '%'));
    ok.then_some(Target::Host(host, port))
}

/// The host and port of a URI ("ipp://h:631/x" → ("h", Some(631))).
pub(crate) fn uri_host(uri: &str) -> Option<(String, Option<u16>)> {
    let rest = uri.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(v6) = authority.strip_prefix('[') {
        let (h, tail) = v6.split_once(']')?;
        return Some((
            h.to_owned(),
            tail.strip_prefix(':').and_then(|p| p.parse().ok()),
        ));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) => Some((h.to_owned(), p.parse().ok())),
        None => Some((authority.to_owned(), None)),
    }
}

fn host_for_uri(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// Whether a TCP port answers within `timeout` (GNOME's JetDirect and LPD probes).
fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs
        .take(4)
        .any(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

/// The outcome of probing a typed address.
pub(crate) struct Probe {
    pub devices: Vec<Value>,
    /// A print server answered but wants a user and password first.
    pub needs_auth: bool,
}

/// An IPP printer at `uri`: its make-and-model, name, location and whether it is driverless; None when nothing
/// answers there as a printer.
fn ipp_printer(
    uri: &str,
    credentials: Option<(&str, &str)>,
) -> Result<Option<Value>, cups::IppError> {
    let (host, port) = uri_host(uri).unwrap_or_default();
    let port = port.unwrap_or(631);
    let conn = Conn::open(
        &Server {
            address: Some((host.clone(), port)),
        },
        credentials,
        5000,
    )?;
    let mut req = conn.request(cups::GET_PRINTER_ATTRIBUTES);
    req.string(cups::OPERATION, cups::URI, "printer-uri", uri);
    req.strings(
        cups::OPERATION,
        cups::KEYWORD,
        "requested-attributes",
        &[
            "printer-make-and-model",
            "printer-info",
            "printer-name",
            "printer-location",
            "printer-device-id",
            "ipp-versions-supported",
            "document-format-supported",
        ],
    );
    let path = uri
        .split_once("://")
        .and_then(|(_, r)| r.find('/').map(|i| &r[i..]))
        .unwrap_or("/");
    match conn.send(req, path) {
        Ok(answer) => {
            let attrs = answer.group(cups::PRINTER);
            if attrs.is_empty() {
                return Ok(None);
            }
            Ok(Some(ipp_device(uri, &attrs)))
        }
        Err(e) if e.status == cups::STATUS_NOT_FOUND || e.status == cups::STATUS_NOT_POSSIBLE => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// A device from an IPP printer's attributes.
pub(crate) fn ipp_device(uri: &str, attrs: &Attrs) -> Value {
    let device_id = text(attrs, "printer-device-id");
    let mut make_model = text(attrs, "printer-make-and-model");
    if make_model.is_empty() {
        let (make, model) = ipp::device_id(&device_id);
        make_model = format!("{make} {model}").trim().to_owned();
    }
    let info = [text(attrs, "printer-info"), text(attrs, "printer-name")]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or_else(|| make_model.clone());
    json!({
        "uri": uri,
        "info": info,
        "makeModel": make_model,
        "deviceId": device_id,
        "location": text(attrs, "printer-location"),
        "class": "network",
        "network": true,
        "source": "address",
        "driverless": ipp::driverless(attrs),
    })
}

/// The shared queues of a CUPS print server (GNOME's "remote CUPS devices"): ipp://host:port/printers/NAME.
fn server_queues(
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<Vec<Value>, cups::IppError> {
    let conn = Conn::open(
        &Server {
            address: Some((host.to_owned(), port)),
        },
        credentials,
        5000,
    )?;
    let mut req = conn.request(cups::CUPS_GET_PRINTERS);
    req.strings(
        cups::OPERATION,
        cups::KEYWORD,
        "requested-attributes",
        &[
            "printer-name",
            "printer-info",
            "printer-location",
            "printer-make-and-model",
            "printer-is-shared",
            "printer-type",
        ],
    );
    let answer = conn.send(req, "/")?;
    Ok(answer
        .groups(cups::PRINTER)
        .iter()
        .filter(|a| cups::one(a, "printer-is-shared") != &json!(false) && !text(a, "printer-name").is_empty())
        .map(|a| {
            let name = text(a, "printer-name");
            json!({
                "uri": format!("ipp://{}:{port}/printers/{}", host_for_uri(host), cups::uri_encode(&name)),
                "info": ([text(a, "printer-info"), name.clone()].into_iter().find(|s| !s.is_empty()).unwrap_or_default()),
                "makeModel": text(a, "printer-make-and-model"),
                "deviceId": "",
                "location": text(a, "printer-location"),
                "class": "network",
                "network": true,
                "source": "address",
                "driverless": true,
                "remoteQueue": true,
            })
        })
        .collect())
}

/// Probe a typed address (GNOME's pp-host.c), with the print server's credentials when the user gave them.
pub(crate) fn probe(target: &Target, credentials: Option<(&str, &str)>) -> Probe {
    let mut devices = Vec::new();
    let mut needs_auth = false;
    let auth = |e: &cups::IppError| {
        e.status == cups::STATUS_NOT_AUTHENTICATED || e.status == cups::STATUS_FORBIDDEN
    };
    match target {
        Target::Uri(uri) => {
            let scheme = uri.split("://").next().unwrap_or("");
            let (host, port) = uri_host(uri).unwrap_or_default();
            let reachable = match scheme {
                "ipp" | "ipps" | "http" | "https" => match ipp_printer(uri, credentials) {
                    Ok(Some(d)) => {
                        devices.push(d);
                        true
                    }
                    Ok(None) => false,
                    Err(e) => {
                        needs_auth |= auth(&e);
                        false
                    }
                },
                "socket" => port_open(&host, port.unwrap_or(9100), Duration::from_secs(3)),
                "lpd" => port_open(&host, port.unwrap_or(515), Duration::from_secs(3)),
                // smb://: typed only (amendment 41), not probed without libsmbclient
                _ => true,
            };
            if reachable && devices.is_empty() {
                devices.push(json!({
                    "uri": uri,
                    "info": host,
                    "makeModel": "",
                    "deviceId": "",
                    "location": "",
                    "class": "network",
                    "network": true,
                    "source": "address",
                    "driverless": false,
                }));
            }
        }
        Target::Host(host, port) => {
            let ipp_port = port.unwrap_or(631);
            match server_queues(host, ipp_port, credentials) {
                Ok(queues) => devices.extend(queues),
                Err(e) => needs_auth |= auth(&e),
            }
            for path in ["ipp/print", "ipp/printer", "ipp"] {
                let uri = format!("ipp://{}:{ipp_port}/{path}", host_for_uri(host));
                if let Ok(Some(d)) = ipp_printer(&uri, credentials) {
                    devices.push(d);
                    break;
                }
            }
            if port.is_none() || *port == Some(9100) {
                if port_open(host, 9100, Duration::from_secs(2)) {
                    devices.push(json!({
                        "uri": format!("socket://{}:9100", host_for_uri(host)),
                        "info": host,
                        "makeModel": "",
                        "deviceId": "",
                        "location": "",
                        "class": "network",
                        "network": true,
                        "source": "address",
                        "driverless": false,
                        "protocol": "jetdirect",
                    }));
                }
            }
            if port.is_none() && port_open(host, 515, Duration::from_secs(2)) {
                devices.push(json!({
                    "uri": format!("lpd://{}/", host_for_uri(host)),
                    "info": host,
                    "makeModel": "",
                    "deviceId": "",
                    "location": "",
                    "class": "network",
                    "network": true,
                    "source": "address",
                    "driverless": false,
                    "protocol": "lpd",
                }));
            }
        }
    }
    Probe {
        devices,
        needs_auth,
    }
}

/// Avahi's IPP printers on the network (`_ipp._tcp`, `_ipps._tcp`), browsed for `window` and resolved.
pub(crate) fn avahi(bus: &gio::DBusConnection, name: &str, window: Duration) -> Vec<Value> {
    use glib::variant::ToVariant;
    use std::sync::{Arc, Mutex};
    let context = glib::MainContext::new();
    let found: Arc<Mutex<Vec<(i32, i32, String, String, String)>>> = Arc::default();
    let out = context.with_thread_default(|| {
        let mut browsers = Vec::new();
        let seen = found.clone();
        // every ItemNew of any browser of ours: the path is known only after ServiceBrowserNew answers
        let sub = bus.signal_subscribe(
            Some(name),
            Some("org.freedesktop.Avahi.ServiceBrowser"),
            Some("ItemNew"),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, path, _, _, params| {
                let iface = params.child_value(0).get::<i32>().unwrap_or(-1);
                let proto = params.child_value(1).get::<i32>().unwrap_or(-1);
                let service = params.child_value(2).str().unwrap_or("").to_owned();
                let kind = params.child_value(3).str().unwrap_or("").to_owned();
                let domain = params.child_value(4).str().unwrap_or("").to_owned();
                let _ = path;
                seen.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((iface, proto, service, kind, domain));
            },
        );
        for kind in ["_ipp._tcp", "_ipps._tcp"] {
            let reply = bus.call_sync(
                Some(name),
                "/",
                "org.freedesktop.Avahi.Server",
                "ServiceBrowserNew",
                Some(&(-1i32, -1i32, kind, "", 0u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            );
            match reply {
                Ok(v) => browsers.push(v.child_value(0).str().unwrap_or("").to_owned()),
                Err(_) => break,
            }
        }
        if !browsers.is_empty() {
            let end = Instant::now() + window;
            while Instant::now() < end {
                while context.iteration(false) {}
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        bus.signal_unsubscribe(sub);
        for path in &browsers {
            let _ = bus.call_sync(
                Some(name),
                path,
                "org.freedesktop.Avahi.ServiceBrowser",
                "Free",
                None,
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            );
        }
        let mut items = found.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // one per service name and type (IPv4 and IPv6, several interfaces)
        items.sort_by(|a, b| (&a.2, &a.3, a.1).cmp(&(&b.2, &b.3, b.1)));
        items.dedup_by(|a, b| a.2 == b.2 && a.3 == b.3);
        let mut devices = Vec::new();
        for (iface, proto, service, kind, domain) in items {
            let Ok(r) = bus.call_sync(
                Some(name),
                "/",
                "org.freedesktop.Avahi.Server",
                "ResolveService",
                Some(
                    &(
                        iface,
                        proto,
                        service.as_str(),
                        kind.as_str(),
                        domain.as_str(),
                        -1i32,
                        0u32,
                    )
                        .to_variant(),
                ),
                None,
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
            ) else {
                continue;
            };
            // (interface, protocol, name, type, domain, host, aprotocol, address, port, txt aay, flags)
            let host = r.child_value(5).str().unwrap_or("").to_owned();
            let port = r.child_value(8).get::<u16>().unwrap_or(631);
            let txt_v = r.child_value(9);
            let entries: Vec<Vec<u8>> = (0..txt_v.n_children())
                .map(|i| {
                    txt_v
                        .child_value(i)
                        .fixed_array::<u8>()
                        .map(<[u8]>::to_vec)
                        .unwrap_or_default()
                })
                .collect();
            devices.push(dnssd_device(
                &service,
                &kind,
                &host,
                port,
                &txt_record(&entries),
            ));
        }
        // a printer that offers ipps and ipp: keep ipps
        let mut keep: Vec<Value> = Vec::new();
        for d in devices {
            let info = d["info"].clone();
            if let Some(existing) = keep.iter_mut().find(|k| k["info"] == info) {
                if d["uri"].as_str().is_some_and(|u| u.starts_with("ipps://")) {
                    *existing = d;
                }
            } else {
                keep.push(d);
            }
        }
        keep
    });
    out.unwrap_or_default()
}

/// One driver of CUPS-Get-PPDs.
pub(crate) fn driver(attrs: &Attrs) -> Value {
    json!({
        "name": text(attrs, "ppd-name"),
        "make": text(attrs, "ppd-make"),
        "makeModel": text(attrs, "ppd-make-and-model"),
        "deviceId": text(attrs, "ppd-device-id"),
        "language": texts(attrs, "ppd-natural-language").first().cloned().unwrap_or_default(),
        "product": texts(attrs, "ppd-product").first().cloned().unwrap_or_default(),
    })
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How well a driver fits a device (0: not at all): its IEEE 1284 id names the same make and model (100), its
/// make-and-model starts with the device's (80), or shares the make and most model words (40-70).
pub(crate) fn score(driver: &Value, device_id: &str, make_model: &str) -> u32 {
    let (dmake, dmodel) = ipp::device_id(device_id);
    let (pmake, pmodel) = ipp::device_id(driver["deviceId"].as_str().unwrap_or(""));
    if !dmodel.is_empty() && !pmodel.is_empty() && norm(&dmodel) == norm(&pmodel) {
        if dmake.is_empty() || pmake.is_empty() || norm(&dmake) == norm(&pmake) {
            return 100;
        }
    }
    let device = if make_model.trim().is_empty() {
        norm(&format!("{dmake} {dmodel}"))
    } else {
        norm(make_model)
    };
    if device.is_empty() {
        return 0;
    }
    let ppd = norm(driver["makeModel"].as_str().unwrap_or(""));
    // the model part before the driver's name (", hpcups 3.23")
    let ppd_model = norm(
        driver["makeModel"]
            .as_str()
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or(""),
    );
    if ppd_model == device || ppd.starts_with(&device) {
        return 80;
    }
    let make = norm(driver["make"].as_str().unwrap_or(""));
    let words: Vec<&str> = device.split(' ').collect();
    if words.is_empty() || make.is_empty() || !(device.starts_with(&make) || norm(&dmake) == make) {
        return 0;
    }
    let model_words: Vec<&&str> = words
        .iter()
        .filter(|w| !make.split(' ').any(|m| m == **w))
        .collect();
    if model_words.is_empty() {
        return 0;
    }
    let hits = model_words
        .iter()
        .filter(|w| ppd_model.split(' ').any(|p| p == ***w))
        .count();
    if hits == 0 {
        return 0;
    }
    40 + (30 * hits / model_words.len()) as u32
}

/// The best drivers for a device, best first (at most five, score 40 and up), `everywhere` first for a driverless
/// device.
pub(crate) fn best(
    drivers: &[Value],
    device_id: &str,
    make_model: &str,
    driverless: bool,
) -> Vec<Value> {
    let mut ranked: Vec<(u32, &Value)> = drivers
        .iter()
        .filter(|d| d["name"] != "everywhere" && d["name"] != "raw")
        .map(|d| (score(d, device_id, make_model), d))
        .filter(|(s, _)| *s >= 40)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1["makeModel"].as_str().cmp(&b.1["makeModel"].as_str()))
    });
    let mut out: Vec<Value> = Vec::new();
    if driverless {
        out.push(json!({"name": "everywhere", "makeModel": "IPP Everywhere", "score": 100, "driverless": true}));
    }
    for (s, d) in ranked.into_iter().take(5) {
        let mut d = d.clone();
        d["score"] = json!(s);
        out.push(d);
    }
    out
}
