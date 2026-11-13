//! The host's Rust services, reached through their existing JSON API: `win11-panel --http-only` (host/src/lib.rs
//! run_http_only) serves `/api/call/<method>` and the `/api/events` stream on 127.0.0.1 with a token, as it does
//! for the WebKit pages. The shell starts that backend as its child (it dies with the shell) unless
//! WIN11_SHELL_BACKEND="port:token" names a running one.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Endpoint = (u16, Arc<String>);

/// The backend's address once it is up: calls made earlier wait for it (D-G67, the shell shows its windows at
/// once instead of after the backend's start, so GNOME's own bar is not seen at login).
#[derive(Default)]
struct Ready {
    endpoint: Mutex<Option<Result<Endpoint, String>>>,
    changed: Condvar,
}

#[derive(Clone)]
pub struct Backend {
    ready: Arc<Ready>,
    lang: Arc<Mutex<String>>,
    _child: Arc<Mutex<Option<Child>>>,
}

impl Backend {
    /// Start the backend (or name a running one) and return at once; `on_ready` runs on the reader thread when
    /// it answers.
    pub fn connect(on_ready: fn()) -> Result<Backend, String> {
        let backend = Backend { ready: Arc::default(), lang: Arc::new(Mutex::new("nl".into())), _child: Arc::default() };
        if let Ok(spec) = std::env::var("WIN11_SHELL_BACKEND") {
            let (port, token) = spec.split_once(':').ok_or("WIN11_SHELL_BACKEND is port:token")?;
            backend.set_ready(Ok((port.parse().map_err(|_| "bad port")?, Arc::new(token.to_owned()))));
            on_ready();
            return Ok(backend);
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let host = exe.with_file_name("win11-panel");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let instance = runtime.join("win11-gnome").join(format!("native-backend-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&instance);
        let mut command = Command::new(&host);
        command.arg("--http-only").env("WIN11_NATIVE_INSTANCE", &instance).env_remove("WIN11_NATIVE_SHELL");
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                // The backend lives exactly as long as the shell.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let child = command.spawn().map_err(|e| format!("{}: {e}", host.display()))?;
        *backend._child.lock().unwrap() = Some(child);
        let waiting = backend.clone();
        std::thread::Builder::new()
            .name("win11-shell-backend".into())
            .spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    if let Ok(text) = std::fs::read_to_string(&instance)
                        && let Ok(v) = serde_json::from_str::<Value>(&text)
                        && let (Some(port), Some(token)) = (v["port"].as_u64(), v["token"].as_str())
                    {
                        waiting.set_ready(Ok((port as u16, Arc::new(token.to_owned()))));
                        on_ready();
                        return;
                    }
                    if Instant::now() > deadline {
                        waiting.set_ready(Err(format!("{} did not start within 30 s", host.display())));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(backend)
    }

    fn set_ready(&self, endpoint: Result<Endpoint, String>) {
        *self.ready.endpoint.lock().unwrap() = Some(endpoint);
        self.ready.changed.notify_all();
    }

    /// The backend's port and token, waiting (off the main thread) until it is up.
    fn endpoint(&self) -> Result<Endpoint, String> {
        let mut guard = self.ready.endpoint.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(35);
        while guard.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("backend not ready".into());
            }
            guard = self.ready.changed.wait_timeout(guard, left).unwrap().0;
        }
        guard.clone().unwrap()
    }

    pub fn set_lang(&self, lang: &str) {
        *self.lang.lock().unwrap() = lang.to_owned();
    }

    /// One API call (blocking; use Shell::call on the main thread).
    pub fn call(&self, method: &str, args: &Value) -> Result<Value, String> {
        let (port, token) = self.endpoint()?;
        let body = json!({"args": args}).to_string();
        let lang = self.lang.lock().unwrap().clone();
        let request = format!(
            "POST /api/call/{method} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nX-Shell-Token: {token}\r\nX-Shell-Lang: {lang}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (status, data) = self.exchange(port, request.as_bytes())?;
        let v: Value = serde_json::from_slice(&data).map_err(|e| format!("{method}: HTTP {status}: {e}"))?;
        if v["ok"] == true {
            Ok(v["result"].clone())
        } else {
            Err(v["error"]["message"].as_str().unwrap_or("error").to_owned())
        }
    }

    /// GET a backend path (icons, thumbnails): "/icon?name=firefox&size=32".
    pub fn get(&self, path_and_query: &str) -> Result<Vec<u8>, String> {
        let (port, token) = self.endpoint()?;
        let sep = if path_and_query.contains('?') { '&' } else { '?' };
        let request = format!(
            "GET {path_and_query}{sep}token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let (status, data) = self.exchange(port, request.as_bytes())?;
        if status == 200 { Ok(data) } else { Err(format!("HTTP {status}")) }
    }

    fn exchange(&self, port: u16, request: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
        stream.write_all(request).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("bad HTTP response")?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut body = raw[split + 4..].to_vec();
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            body = dechunk(&body);
        }
        Ok((status, body))
    }

    /// The event stream: `on(name, data)` from a reader thread, reconnecting after errors.
    pub fn events(&self, on: fn(String, Value)) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("win11-shell-events".into())
            .spawn(move || loop {
                let _ = me.read_events(on);
                std::thread::sleep(Duration::from_millis(1500));
            })
            .ok();
    }

    fn read_events(&self, on: fn(String, Value)) -> Result<(), String> {
        let (port, token) = self.endpoint()?;
        let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        let request = format!(
            "GET /api/events?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        // headers
        loop {
            line.clear();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("closed".into());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let (mut event, mut data) = (String::new(), String::new());
        loop {
            line.clear();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("closed".into());
            }
            let l = line.trim_end_matches(['\r', '\n']);
            if l.is_empty() {
                if !event.is_empty() {
                    let value = serde_json::from_str(&data).unwrap_or(Value::Null);
                    on(std::mem::take(&mut event), value);
                }
                data.clear();
            } else if let Some(v) = l.strip_prefix("event:") {
                event = v.trim().to_owned();
            } else if let Some(v) = l.strip_prefix("data:") {
                data.push_str(v.trim_start());
            }
        }
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..i]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = i + 2;
        if rest.len() < start + size {
            break;
        }
        out.extend_from_slice(&rest[start..start + size]);
        rest = &rest[(start + size + 2).min(rest.len())..];
    }
    out
}
