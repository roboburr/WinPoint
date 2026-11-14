//! The host's Rust services, reached through their existing JSON API: `win11-panel --http-only` (host/src/lib.rs
//! run_http_only) serves `/api/call/<method>` and the `/api/events` stream on 127.0.0.1 with a token. The shell
//! starts that backend as its child (it dies with the shell) unless WIN11_SHELL_BACKEND="port:token" names a
//! running one. In a systemd session the child moves a copy of itself
//! into the shell slice (host process_scope::ensure_native) and exits; that copy writes the instance file with its
//! own pid, and the shell follows it through a pidfd.
//!
//! A supervisor thread (alive as long as the shell, so PR_SET_PDEATHSIG of the backends it starts holds) waits on
//! the backend. It ends after leaving `<instance>.restart` (host lib.rs `restart_note`, exit status
//! [`RESTART_EXIT`]): for a `shell.restartAll` the main thread execs win11-shell again (main.rs `restart`: the new
//! build, a new backend), for a plain `shell.restart` a new backend starts. Any other end: a new backend after
//! 0.5 s, 1 s, 2 s, ... (at most [`MAX_RESPAWNS`] in a row that end within a minute); calls wait meanwhile, and
//! "backend.ready" comes again. A call under way does not time out while its backend lives.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Endpoint = (u16, Arc<String>);

/// The exit status of a backend that restarts (host lib.rs `RESTART_EXIT`); it wrote `<instance>.restart` first.
pub const RESTART_EXIT: i32 = 75;
/// Backends in a row that may end within [`STABLE`] before the shell stays offline.
pub const MAX_RESPAWNS: u32 = 5;
const STABLE: Duration = Duration::from_secs(60);
/// Without an answer for this long, a call checks that its backend is still there.
const QUIET: Duration = Duration::from_secs(15);

/// The backend's address once it is up: calls made earlier wait for it (D-G67, the shell shows its windows at
/// once instead of after the backend's start, so GNOME's own bar is not seen at login).
#[derive(Default)]
struct Ready {
    endpoint: Mutex<Option<Result<Endpoint, String>>>,
    changed: Condvar,
}

/// What a restarting backend left in `<instance>.restart` (host lib.rs `restart_note`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestartNote {
    /// `shell.restartAll` (false: a plain `shell.restart`)
    pub all: bool,
    /// an activation token for the new windows, if the backend asked for one
    pub token: Option<String>,
}

impl RestartNote {
    fn parse(v: &Value, pid: u32) -> Option<RestartNote> {
        if v["pid"].as_u64() != Some(u64::from(pid)) {
            return None;
        }
        let token = v["token"]
            .as_str()
            .filter(|t| !t.is_empty() && t.len() <= 4096 && !t.contains('\0'))
            .map(str::to_owned);
        Some(RestartNote { all: v["all"] == true, token })
    }
}

/// The supervisor's side of a restart.
#[derive(Default)]
struct Life {
    note: Mutex<Option<RestartNote>>,
    answer: Mutex<Option<bool>>,
    answered: Condvar,
}

#[derive(Clone)]
pub struct Backend {
    ready: Arc<Ready>,
    lang: Arc<Mutex<String>>,
    life: Arc<Life>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn note_path(instance: &Path) -> PathBuf {
    let mut name = instance.as_os_str().to_owned();
    name.push(".restart");
    PathBuf::from(name)
}

/// The endpoint and the backend's pid in an instance file.
fn read_instance(path: &Path) -> Option<(Endpoint, u32)> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let port = u16::try_from(v["port"].as_u64()?).ok().filter(|p| *p > 0)?;
    let token = v["token"].as_str().filter(|t| !t.is_empty())?.to_owned();
    let pid = u32::try_from(v["pid"].as_u64()?).ok()?;
    Some(((port, Arc::new(token)), pid))
}

/// The delay before the `n`th new backend in a row (1-based): 0.5 s, 1 s, 2 s, 4 s, 8 s.
fn respawn_delay(n: u32) -> Duration {
    Duration::from_millis(500u64 << n.saturating_sub(1).min(4))
}

/// Block until the process `pid` (not necessarily a child) has ended.
fn wait_gone(pid: u32) {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if fd < 0 {
        // ESRCH: gone already; anything else: look every second
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        return;
    }
    let fd = fd as libc::c_int;
    let mut poll = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    while unsafe { libc::poll(&mut poll, 1, -1) } < 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
    {}
    unsafe { libc::close(fd) };
}

struct Spawner {
    host: PathBuf,
    instance: PathBuf,
}

impl Spawner {
    fn spawn(&self) -> Result<Child, String> {
        let _ = std::fs::remove_file(&self.instance);
        let _ = std::fs::remove_file(note_path(&self.instance));
        let mut command = Command::new(&self.host);
        command.arg("--http-only").env("WIN11_NATIVE_INSTANCE", &self.instance).env_remove("WIN11_NATIVE_SHELL");
        let parent = std::process::id();
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(move || {
                // The backend lives exactly as long as the thread that starts it: the main thread for the first
                // one, the supervisor (alive as long as the shell) for every later one.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if libc::getppid() as u32 != parent {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
        command.spawn().map_err(|e| format!("{}: {e}", self.host.display()))
    }

    /// Wait until the backend wrote its instance file (at most 30 s): the endpoint and the backend's pid (the
    /// child's, or that of the copy it moved into the shell slice, after which the child exits with 0).
    fn wait_ready(&self, child: &mut Child) -> Result<(Endpoint, u32), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(found) = read_instance(&self.instance) {
                return Ok(found);
            }
            if let Ok(Some(status)) = child.try_wait()
                && !status.success()
            {
                return Err(format!("{} exited ({status})", self.host.display()));
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} did not start within 30 s", self.host.display()));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn take_note(&self, pid: u32) -> Option<RestartNote> {
        let path = note_path(&self.instance);
        let bytes = std::fs::read(&path).ok();
        let _ = std::fs::remove_file(&path);
        RestartNote::parse(&serde_json::from_slice(&bytes?).ok()?, pid)
    }
}

impl Backend {
    /// Start the backend (or name a running one) and return at once; `on_ready` runs on the reader thread when
    /// it answers (a later backend sends "backend.ready" again).
    pub fn connect(on_ready: fn()) -> Result<Backend, String> {
        let backend = Backend { ready: Arc::default(), lang: Arc::new(Mutex::new("nl".into())), life: Arc::default() };
        if let Ok(spec) = std::env::var("WIN11_SHELL_BACKEND") {
            let (port, token) = spec.split_once(':').ok_or("WIN11_SHELL_BACKEND is port:token")?;
            backend.set_ready(Ok((port.parse().map_err(|_| "bad port")?, Arc::new(token.to_owned()))));
            on_ready();
            return Ok(backend);
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let spawner = Spawner {
            host: exe.with_file_name("win11-panel"),
            instance: runtime.join("win11-gnome").join(format!("native-backend-{}.json", std::process::id())),
        };
        let child = spawner.spawn()?;
        let supervisor = backend.clone();
        std::thread::Builder::new()
            .name("win11-shell-backend".into())
            .spawn(move || supervisor.supervise(spawner, child, on_ready))
            .map_err(|e| e.to_string())?;
        Ok(backend)
    }

    fn supervise(&self, spawner: Spawner, first: Child, on_ready: fn()) {
        let mut first_ready = true;
        let mut next = Some(first);
        let mut failures = 0u32;
        loop {
            let mut child = match next.take().map(Ok).unwrap_or_else(|| spawner.spawn()) {
                Ok(child) => child,
                Err(error) => {
                    eprintln!("win11-shell: backend: {error}");
                    if !self.wait_before_respawn(&mut failures) {
                        return;
                    }
                    continue;
                }
            };
            match spawner.wait_ready(&mut child) {
                Ok((endpoint, pid)) => {
                    let up = Instant::now();
                    self.set_ready(Ok(endpoint));
                    if std::mem::take(&mut first_ready) {
                        on_ready();
                    } else {
                        crate::core::dispatch("backend.ready".into(), json!({ "respawn": true }));
                    }
                    let status = if pid == child.id() {
                        match child.wait() {
                            Ok(s) if s.code() == Some(RESTART_EXIT) => format!("{s}, a restart without its note"),
                            Ok(s) => s.to_string(),
                            Err(e) => e.to_string(),
                        }
                    } else {
                        // the copy in the shell slice; the child itself has ended (reap it)
                        let _ = child.wait();
                        wait_gone(pid);
                        "ended".into()
                    };
                    *lock(&self.ready.endpoint) = None;
                    let _ = std::fs::remove_file(&spawner.instance);
                    if let Some(note) = spawner.take_note(pid) {
                        if note.all && !self.restart(note) {
                            return;
                        }
                        failures = 0;
                        continue;
                    }
                    eprintln!("win11-shell: the backend ended ({status}); starting a new one");
                    if up.elapsed() >= STABLE {
                        failures = 0;
                    }
                }
                Err(error) => eprintln!("win11-shell: backend: {error}"),
            }
            if !self.wait_before_respawn(&mut failures) {
                return;
            }
        }
    }

    fn wait_before_respawn(&self, failures: &mut u32) -> bool {
        *failures += 1;
        if *failures > MAX_RESPAWNS {
            eprintln!("win11-shell: the backend failed {MAX_RESPAWNS} times in a row; staying offline");
            self.set_ready(Err("the backend keeps failing".into()));
            return false;
        }
        std::thread::sleep(respawn_delay(*failures));
        true
    }

    /// shell.restartAll: the main thread execs the shell again ("backend.restart", main.rs). True: it could not,
    /// start a new backend.
    fn restart(&self, note: RestartNote) -> bool {
        *lock(&self.life.note) = Some(note);
        *lock(&self.life.answer) = None;
        crate::core::dispatch("backend.restart".into(), Value::Null);
        let mut answer = lock(&self.life.answer);
        let deadline = Instant::now() + Duration::from_secs(30);
        while answer.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            answer = self.life.answered.wait_timeout(answer, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        answer.take().unwrap_or(true)
    }

    /// The note of the backend that restarted (main.rs, on "backend.restart"); taken once.
    pub fn take_restart(&self) -> Option<RestartNote> {
        lock(&self.life.note).take()
    }

    /// The answer to "backend.restart": `respawn` true starts a new backend (the exec failed).
    pub fn answer_restart(&self, respawn: bool) {
        *lock(&self.life.answer) = Some(respawn);
        self.life.answered.notify_all();
    }

    fn set_ready(&self, endpoint: Result<Endpoint, String>) {
        *lock(&self.ready.endpoint) = Some(endpoint);
        self.ready.changed.notify_all();
    }

    /// The backend's port and token, waiting (off the main thread) until it is up.
    fn endpoint(&self) -> Result<Endpoint, String> {
        let mut guard = lock(&self.ready.endpoint);
        let deadline = Instant::now() + Duration::from_secs(35);
        while guard.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("backend not ready".into());
            }
            guard = self.ready.changed.wait_timeout(guard, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        guard.clone().unwrap_or_else(|| Err("backend not ready".into()))
    }

    pub fn set_lang(&self, lang: &str) {
        *lock(&self.lang) = lang.to_owned();
    }

    /// One API call (blocking; use Shell::call on the main thread).
    pub fn call(&self, method: &str, args: &Value) -> Result<Value, String> {
        let (port, token) = self.endpoint()?;
        let body = json!({"args": args}).to_string();
        let lang = lock(&self.lang).clone();
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

    /// The backend at `port` is still the one to talk to, and it accepts a connection.
    fn still_there(&self, port: u16) -> bool {
        matches!(&*lock(&self.ready.endpoint), Some(Ok((current, _))) if *current == port)
            && TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(3)).is_ok()
    }

    fn exchange(&self, port: u16, request: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_read_timeout(Some(QUIET)).ok();
        stream.write_all(request).map_err(|e| e.to_string())?;
        // a call under way does not time out while its backend lives; a backend that is gone ends it
        let mut raw = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if !self.still_there(port) {
                        return Err("the backend is gone".into());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("bad HTTP response")?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut body = raw[split + 4..].to_vec();
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            body = dechunk(&body);
        }
        Ok((status, body))
    }

    /// The event stream: `on(name, data)` from a reader thread, reconnecting after errors (and to a new backend).
    pub fn events(&self, on: fn(String, Value)) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("win11-shell-events".into())
            .spawn(move || loop {
                // Ok: the stream was up and its backend ended (a restart, a new backend): back soon
                let wait = if me.read_events(on).is_ok() { 300 } else { 1500 };
                if matches!(&*lock(&me.ready.endpoint), Some(Err(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(wait));
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
            // the stream was up: its end is Ok (the events thread reconnects at once)
            if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) {
                return Ok(());
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
