//! snapd's Unix REST protocol. Authentication belongs to the OS polkit agent.
use super::common::*;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};
fn read_response(stream: UnixStream) -> Result<Value, String> {
    let mut r = BufReader::new(stream);
    let status = limited_line(&mut r, 4096)?;
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("Invalid snapd HTTP status")?;
    let (mut length, mut chunked) = (None, false);
    let mut headers = 0;
    loop {
        let line = limited_line(&mut r, 65536)?;
        headers += line.len();
        if headers > 65536 {
            return Err("snapd response headers too large".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if line.is_empty() {
            return Err("Incomplete snapd response".into());
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse::<usize>().ok()
            }
            if k.eq_ignore_ascii_case("transfer-encoding") {
                chunked = v.to_ascii_lowercase().contains("chunked")
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let line = limited_line(&mut r, 128)?;
            let n = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| "Invalid snapd chunk")?;
            if n == 0 {
                break;
            }
            if body.len().saturating_add(n) > 4 << 20 {
                return Err("snapd response too large".into());
            }
            let old = body.len();
            body.resize(old + n, 0);
            r.read_exact(&mut body[old..]).map_err(|e| e.to_string())?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf).map_err(|e| e.to_string())?;
            if crlf != *b"\r\n" {
                return Err("Invalid snapd chunk boundary".into());
            }
        }
    } else {
        let max = length.unwrap_or(4 << 20);
        if max > 4 << 20 {
            return Err("snapd response too large".into());
        }
        if let Some(n) = length {
            body.resize(n, 0);
            r.read_exact(&mut body).map_err(|e| e.to_string())?;
        } else {
            r.take(max as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
        }
        if body.len() > max {
            return Err("snapd response too large".into());
        }
        if length.is_some_and(|n| body.len() != n) {
            return Err("Incomplete snapd response".into());
        }
    }
    let value: Value = serde_json::from_slice(&body).map_err(|_| "Invalid snapd JSON")?;
    if code >= 400 || value["type"] == "error" {
        return Err(format!(
            "daemon_error: {}",
            value["result"]["message"]
                .as_str()
                .map(|s| clean(s, 4000))
                .unwrap_or_else(|| format!("snapd HTTP {code}"))
        ));
    }
    Ok(value)
}
fn connect(socket: &Path) -> Result<UnixStream, String> {
    let s = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(1800)))
        .map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| e.to_string())?;
    Ok(s)
}
fn send(s: &mut UnixStream, mut bytes: &[u8], cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let begin = Instant::now();
    while !bytes.is_empty() {
        if cancel() {
            return Err("cancelled_upload".into());
        }
        match s.write(bytes) {
            Ok(0) => return Err("snapd connection closed".into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) && begin.elapsed() < Duration::from_secs(1800) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
pub(super) fn request(
    socket: &Path,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let mut stream = connect(socket)?;
    let bytes = body.map(|v| v.to_string().into_bytes()).unwrap_or_default();
    let header = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Allow-Interaction: true\r\n\r\n",
        bytes.len()
    );
    send(&mut stream, header.as_bytes(), &|| false)?;
    send(&mut stream, &bytes, &|| false)?;
    read_response(stream).map_err(|e| {
        if method == "POST" {
            mutation_error(e)
        } else {
            e
        }
    })
}
pub(super) fn installed(socket: &Path, name: &str) -> Result<Value, String> {
    if !snap_name(name) {
        return Err("Invalid snap name".into());
    }
    let value = request(socket, "GET", &format!("/v2/snaps/{name}"), None)?;
    if !["active", "installed"].contains(&value["result"]["status"].as_str().unwrap_or("")) {
        return Err("The snap is not installed".into());
    }
    Ok(value["result"].clone())
}
pub(super) fn install(
    socket: &Path,
    path: &Path,
    confinement: &str,
    cancel: &dyn Fn() -> bool,
    update: &dyn Fn(u32),
) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    let boundary = format!("win11-{}", id().map_err(|e| e.to_string())?);
    let mut before = String::new();
    for (k, v) in [("action", "install"), ("dangerous", "true")]
        .into_iter()
        .chain(if confinement == "classic" {
            Some(("classic", "true"))
        } else if confinement == "devmode" {
            Some(("devmode", "true"))
        } else {
            None
        })
    {
        before.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
        ))
    }
    before.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"snap\"; filename=\"package.snap\"\r\nContent-Type: application/octet-stream\r\n\r\n"));
    let after = format!("\r\n--{boundary}--\r\n");
    let length = (before.len() as u64)
        .checked_add(size)
        .and_then(|n| n.checked_add(after.len() as u64))
        .ok_or("Package too large")?;
    let mut stream = connect(socket)?;
    let header = format!(
        "POST /v2/snaps HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {length}\r\nX-Allow-Interaction: true\r\n\r\n"
    );
    send(&mut stream, header.as_bytes(), cancel)?;
    send(&mut stream, before.as_bytes(), cancel)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        send(&mut stream, &buffer[..n], cancel)?;
        done += n as u64;
        update(((done.saturating_mul(20)) / size.max(1)) as u32)
    }
    if done != size {
        return Err("The staged package changed".into());
    }
    send(&mut stream, after.as_bytes(), cancel)?; // Once the complete request is sent, retain the response/change id even after a cancel request.
    let reply = read_response(stream).map_err(mutation_error)?;
    change(&reply)
}
fn change(v: &Value) -> Result<String, String> {
    v["change"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_digit()))
        .map(String::from)
        .ok_or_else(|| "outcome_unknown: snapd did not return a change id; check installed software before retrying".into())
}
pub(super) fn remove(socket: &Path, name: &str) -> Result<String, String> {
    if !snap_name(name) {
        return Err("Invalid snap name".into());
    }
    change(&request(
        socket,
        "POST",
        &format!("/v2/snaps/{name}"),
        Some(&json!({"action":"remove","purge":false})),
    )?)
}
pub(super) enum Outcome {
    Done,
    Cancelled,
}
pub(super) fn wait(
    socket: &Path,
    change_id: &str,
    cancel: &dyn Fn() -> bool,
    update: &dyn Fn(Option<u32>, bool),
) -> Result<Outcome, String> {
    if change_id.is_empty() || !change_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid snap change".into());
    }
    let begin = Instant::now();
    let mut abort = false;
    loop {
        let v = request(socket, "GET", &format!("/v2/changes/{change_id}"), None)
            .map_err(|e| format!("outcome_unknown: snapd change {change_id}: {e}"))?;
        let result = &v["result"];
        if !result.is_object()
            || result["ready"].as_bool().is_none()
            || result["status"].as_str().is_none()
        {
            return Err("outcome_unknown: snapd returned an invalid change state".into());
        }
        let ready = result["ready"] == true;
        let status = result["status"].as_str().unwrap_or("");
        let (mut done, mut total) = (0u64, 0u64);
        if let Some(tasks) = result["tasks"].as_array() {
            for task in tasks {
                done = done.saturating_add(task["progress"]["done"].as_u64().unwrap_or(0));
                total = total.saturating_add(task["progress"]["total"].as_u64().unwrap_or(0));
            }
        }
        update(
            (total > 0).then(|| 20 + ((done.min(total) * 80) / total) as u32),
            !ready,
        );
        if ready {
            if status == "Done" {
                return Ok(Outcome::Done);
            }
            if abort && ["Undone", "Abort"].contains(&status) {
                return Ok(Outcome::Cancelled);
            }
            return Err(result["err"]
                .as_str()
                .map(|s| clean(s, 4000))
                .unwrap_or_else(|| format!("snapd change ended with status {status}")));
        }
        if cancel() && !abort {
            if request(
                socket,
                "POST",
                &format!("/v2/changes/{change_id}"),
                Some(&json!({"action":"abort"})),
            )
            .is_ok()
            {
                abort = true
            }
        }
        if begin.elapsed() > Duration::from_secs(2 * 3600) {
            return Err("snapd change did not finish; its result is unknown".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn limited_line(r: &mut impl BufRead, max: usize) -> Result<String, String> {
    let mut result = Vec::new();
    loop {
        let b = r.fill_buf().map_err(|e| e.to_string())?;
        if b.is_empty() {
            break;
        }
        let n = b
            .iter()
            .position(|v| *v == b'\n')
            .map_or(b.len(), |n| n + 1);
        if result.len().saturating_add(n) > max {
            return Err("snapd response line too large".into());
        }
        let done = b[n - 1] == b'\n';
        result.extend_from_slice(&b[..n]);
        r.consume(n);
        if done {
            break;
        }
    }
    String::from_utf8(result).map_err(|_| "Invalid snapd response header".into())
}

fn mutation_error(e: String) -> String {
    e.strip_prefix("daemon_error: ")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("outcome_unknown: snapd did not return a reliable result: {e}"))
}
