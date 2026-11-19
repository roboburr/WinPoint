//! snapd's REST API over its Unix socket for "Vereiste machtigingen" of a snap (GNOME Settings' CcSnapdClient):
//! `GET /v2/connections?select=all` for the plugs and the slots they can connect to, `POST /v2/interfaces` to
//! connect or disconnect one, then `GET /v2/changes/<id>` until snapd is done. Writes send `X-Allow-Interaction:
//! true` (amendment 42), so snapd itself asks for the password through polkit (io.snapcraft.snapd.manage-interfaces).
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) const SOCKET: &str = "/run/snapd.socket";
const MAX_RESPONSE: usize = 8 << 20;
/// Interfaces GNOME Settings leaves out of the list (cc-snap-row.c).
const HIDDEN: &[&str] = &["content", "desktop-legacy", "unity7", "unity8"];

/// A snap name (snapd's rule: lowercase letters, digits and dashes, at most 40, not starting or ending with a
/// dash); an instance key (`name_key`) is allowed too.
pub(super) fn name_ok(name: &str) -> bool {
    let (base, key) = name.split_once('_').unwrap_or((name, ""));
    let part = |s: &str, max: usize| {
        !s.is_empty()
            && s.len() <= max
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !s.starts_with('-')
            && !s.ends_with('-')
    };
    part(base, 40)
        && (key.is_empty()
            || (key.len() <= 10
                && key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())))
}

/// A plug or slot name.
pub(super) fn port_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A failed request: snapd's status code, its error kind and message.
#[derive(Debug, PartialEq)]
pub(super) struct SnapError {
    pub status: u16,
    pub kind: String,
    pub message: String,
}

impl SnapError {
    fn local(message: impl Into<String>) -> SnapError {
        SnapError {
            status: 0,
            kind: String::new(),
            message: message.into(),
        }
    }
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, SnapError> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| SnapError::local("invalid chunk"))?;
        let size = std::str::from_utf8(&body[..end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .ok_or_else(|| SnapError::local("invalid chunk size"))?;
        body = &body[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 || &body[size..size + 2] != b"\r\n" {
            return Err(SnapError::local("incomplete chunk"));
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// An HTTP/1.1 answer of snapd (the whole connection's bytes): its JSON, or the error it reports.
pub(super) fn parse_response(raw: &[u8]) -> Result<Value, SnapError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| SnapError::local("incomplete response"))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| SnapError::local("invalid status line"))?;
    let chunked = lines.any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
        })
    });
    let body = &raw[split + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    let value: Value = serde_json::from_slice(&body).map_err(|_| SnapError {
        status,
        ..SnapError::local("invalid JSON")
    })?;
    if status >= 400 || value["type"] == "error" {
        return Err(SnapError {
            status: value["status-code"].as_u64().map_or(status, |s| s as u16),
            kind: value["result"]["kind"].as_str().unwrap_or("").to_owned(),
            message: value["result"]["message"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect(),
        });
    }
    Ok(value)
}

/// One request over the socket; `wait` is how long snapd may take (a write waits for the password dialog).
pub(super) fn request(
    socket: &Path,
    method: &str,
    path: &str,
    body: Option<&Value>,
    wait: Duration,
) -> Result<Value, SnapError> {
    let mut stream = UnixStream::connect(socket).map_err(|e| SnapError::local(e.to_string()))?;
    stream
        .set_read_timeout(Some(wait))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(5))))
        .map_err(|e| SnapError::local(e.to_string()))?;
    let bytes = body.map(|v| v.to_string().into_bytes()).unwrap_or_default();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if body.is_some() {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\nX-Allow-Interaction: true\r\n",
            bytes.len()
        ));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|e| SnapError::local(e.to_string()))?;
    let mut raw = Vec::new();
    stream
        .take(MAX_RESPONSE as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| SnapError::local(e.to_string()))?;
    if raw.len() > MAX_RESPONSE {
        return Err(SnapError::local("response too large"));
    }
    parse_response(&raw)
}

/// The plugs of `snap` with the slots each can connect to (`/v2/connections?select=all`'s result), as privacy.app
/// shows them: hidden interfaces and plugs with nothing to connect to are left out.
pub(super) fn plugs(result: &Value, snap: &str) -> Vec<Value> {
    let empty = Vec::new();
    let slots = result["slots"].as_array().unwrap_or(&empty);
    let mut out = Vec::new();
    for plug in result["plugs"].as_array().unwrap_or(&empty) {
        let interface = plug["interface"].as_str().unwrap_or("");
        if plug["snap"] != snap || HIDDEN.contains(&interface) {
            continue;
        }
        let candidates: Vec<Value> = slots
            .iter()
            .filter(|s| s["interface"] == interface)
            .map(|s| json!({ "snap": s["snap"], "slot": s["slot"] }))
            .collect();
        let connected = plug["connections"]
            .as_array()
            .and_then(|c| c.first())
            .map(|c| json!({ "snap": c["snap"], "slot": c["slot"] }));
        if candidates.is_empty() && connected.is_none() {
            continue;
        }
        out.push(json!({
            "plug": plug["plug"],
            "interface": interface,
            "connected": connected.is_some(),
            "slot": connected.unwrap_or(Value::Null),
            "slots": candidates,
        }));
    }
    out.sort_by(|a, b| {
        a["interface"]
            .as_str()
            .cmp(&b["interface"].as_str())
            .then(a["plug"].as_str().cmp(&b["plug"].as_str()))
    });
    out
}

/// The body of `POST /v2/interfaces`.
pub(super) fn connect_body(
    connect: bool,
    snap: &str,
    plug: &str,
    slot_snap: &str,
    slot: &str,
) -> Value {
    json!({
        "action": if connect { "connect" } else { "disconnect" },
        "plugs": [{ "snap": snap, "plug": plug }],
        "slots": [{ "snap": slot_snap, "slot": slot }],
    })
}

/// Wait until change `id` is ready: Ok when it is Done, else snapd's error text.
pub(super) fn wait_change(socket: &Path, id: &str, limit: Duration) -> Result<(), SnapError> {
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SnapError::local("no change id"));
    }
    let start = Instant::now();
    loop {
        let v = request(
            socket,
            "GET",
            &format!("/v2/changes/{id}"),
            None,
            Duration::from_secs(30),
        )?;
        let result = &v["result"];
        if result["ready"] == true {
            return match result["status"].as_str() {
                Some("Done") => Ok(()),
                status => Err(SnapError::local(
                    result["err"]
                        .as_str()
                        .map(|s| s.chars().take(400).collect::<String>())
                        .unwrap_or_else(|| {
                            format!("snapd change ended as {}", status.unwrap_or("?"))
                        }),
                )),
            };
        }
        if start.elapsed() > limit {
            return Err(SnapError::local("snapd did not finish in time"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
