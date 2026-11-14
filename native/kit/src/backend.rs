//! The host's Rust services through their JSON API (native/shell/src/backend.rs, generalised): the app starts
//! `win11-app --app <kind> --http-only` from its own directory as a child with WIN11_NATIVE_INSTANCE=<file>; the
//! backend writes `{port, token, pid}` there and serves `POST /api/call/<method>` (`{"args"}` -> `{"ok","result"|"error"}`,
//! headers X-Shell-Token and X-Shell-Lang), the SSE stream `GET /api/events?token=` and GET paths such as
//! `/icon?name=..&size=..`, `/icon?mime=..`, `/thumb?path=..&size=..` and `/file?path=..`. The child gets
//! PR_SET_PDEATHSIG, so it dies with the app. WIN11_APP_BACKEND="port:token" names a running backend instead.
//!
//! `Backend` is `Send + Sync + Clone`: its blocking `call` and `get` may be used from worker threads. Calls made
//! before the backend answers wait for it (the Ready condvar), at most 35 s. A call that is under way never times
//! out while the backend lives (a big copy, an extraction): every 15 s without an answer the client checks that the
//! same backend still accepts connections, and only a backend that is gone ends the call with "offline".
//!
//! # Lifecycle
//!
//! One supervisor thread ("win11-kit-backend", alive as long as the app, so PR_SET_PDEATHSIG of the backends it
//! starts holds) waits on the backend:
//! * it exits after leaving `<instance>.restart` (host lib.rs `restart_note`, exit status [`RESTART_EXIT`]): a
//!   `shell.restartAll` (or `shell.restart`) whose waits are over. The main thread decides (kit app.rs
//!   `backend_restarted`): the app saves its state and execs itself (the new build, a new backend), quits (no
//!   window), or keeps running on a new backend.
//! * any other exit: a new backend, after 0.5 s, 1 s, 2 s, ... (at most [`MAX_RESPAWNS`] in a row that end
//!   within a minute); meanwhile calls wait and the event stream is offline ("bridge.status"), then
//!   "backend.ready" {respawn: true} comes again. After that many the client stays offline.
//!
//! The instance file stays while its backend runs (diagnostics, the harness) and goes when it ends.
use serde_json::{Value, json};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Endpoint = (u16, Arc<String>);

/// The exit status of a backend that restarts (host lib.rs `RESTART_EXIT`, EX_TEMPFAIL); it wrote
/// `<instance>.restart` first.
pub const RESTART_EXIT: i32 = 75;
/// Backends in a row that may end within [`STABLE`] before the client gives up.
pub const MAX_RESPAWNS: u32 = 5;
/// A backend that ran this long resets the count of failed starts.
const STABLE: Duration = Duration::from_secs(60);
/// Without an answer for this long, a call checks that its backend is still there.
const QUIET: Duration = Duration::from_secs(15);

/// A failed backend (or extension) call: the host's error code ("offline", "untrusted", "not_found",
/// "cancelled", ...), its message in the UI language (show this one), the catalogue key of that message when the
/// host has one ("system.bt_not_responding", "update.err.offline", ...: match on the key or the code, never on the
/// message) and the optional data the host sent. The host's error JSON (host files/mod.rs `ApiError::json`, contracts
/// C2) is {code, message, key?, params?, data?}.
#[derive(Clone, Debug, PartialEq)]
pub struct CallError {
    pub code: String,
    pub message: String,
    pub key: Option<String>,
    pub data: Value,
}

impl CallError {
    pub fn new(code: &str, message: impl Into<String>) -> CallError {
        CallError { code: code.to_owned(), message: message.into(), key: None, data: Value::Null }
    }

    /// The error of a host or extension answer `{"ok": false, "error": e}`; `message` when `e` has none.
    pub fn from_json(e: &Value, message: &str) -> CallError {
        CallError {
            code: e["code"].as_str().unwrap_or("internal").to_owned(),
            message: e["message"].as_str().map(str::to_owned).unwrap_or_else(|| message.to_owned()),
            key: e["key"].as_str().filter(|k| !k.is_empty()).map(str::to_owned),
            data: e.get("data").cloned().unwrap_or(Value::Null),
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CallError {}

impl From<CallError> for String {
    fn from(e: CallError) -> String {
        e.message
    }
}

/// What a restarting backend left in `<instance>.restart` (host lib.rs `restart_note`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestartNote {
    /// `shell.restartAll` (false: a plain `shell.restart` of this backend)
    pub all: bool,
    /// this app's backend called `shell.restartAll` (Settings reopens on its update page)
    pub origin: bool,
    /// a fresh activation token for the new window (XDG_ACTIVATION_TOKEN / DESKTOP_STARTUP_ID)
    pub token: Option<String>,
}

impl RestartNote {
    /// The note of the backend `pid`, if it is one (its pid; a token without NUL, at most 4 KiB).
    pub(crate) fn parse(v: &Value, pid: u32) -> Option<RestartNote> {
        if v["pid"].as_u64() != Some(u64::from(pid)) {
            return None;
        }
        let token = v["token"]
            .as_str()
            .filter(|t| !t.is_empty() && t.len() <= 4096 && !t.contains('\0'))
            .map(str::to_owned);
        Some(RestartNote { all: v["all"] == true, origin: v["origin"] == true, token })
    }
}

/// `<instance>.restart` (host lib.rs `restart_note_path`).
pub(crate) fn restart_note_path(instance: &Path) -> PathBuf {
    let mut name = instance.as_os_str().to_owned();
    name.push(".restart");
    PathBuf::from(name)
}

/// The endpoint in an instance file, if the backend `pid` wrote it.
pub(crate) fn read_instance(path: &Path, pid: u32) -> Option<(u16, String)> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if v["pid"].as_u64() != Some(u64::from(pid)) {
        return None;
    }
    let port = u16::try_from(v["port"].as_u64()?).ok().filter(|p| *p > 0)?;
    Some((port, v["token"].as_str().filter(|t| !t.is_empty())?.to_owned()))
}

/// The delay before the `n`th new backend in a row (1-based): 0.5 s, 1 s, 2 s, 4 s, 8 s.
pub(crate) fn respawn_delay(n: u32) -> Duration {
    Duration::from_millis(500u64 << n.saturating_sub(1).min(4))
}

#[derive(Default)]
struct Ready {
    endpoint: Mutex<Option<Result<Endpoint, String>>>,
    changed: Condvar,
}

/// The supervisor's side of a restart, and the files of the running backend.
#[derive(Default)]
struct Life {
    /// the note of a backend that restarted, until the main thread takes it
    note: Mutex<Option<RestartNote>>,
    /// the main thread's answer: Some(true) a new backend, Some(false) the app execs itself or quits
    answer: Mutex<Option<bool>>,
    answered: Condvar,
    /// the instance file of the backend this client started (None: WIN11_APP_BACKEND)
    instance: Mutex<Option<PathBuf>>,
}

/// The backend client. Cheap to clone; one per process (`kit::ctx().backend()`).
#[derive(Clone)]
pub struct Backend {
    ready: Arc<Ready>,
    lang: Arc<Mutex<String>>,
    connected: Arc<std::sync::atomic::AtomicBool>,
    life: Arc<Life>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Texts of the bridge (last-webkit:vendor/ui/js/core/bridge.js), by language.
fn text(key: &str, en: bool) -> &'static str {
    match (key, en) {
        ("offline", false) => "Geen verbinding met Win11 Shell. Probeer het zo opnieuw.",
        ("offline", true) => "No connection to Win11 Shell. Try again in a moment.",
        ("unauthorized", false) => "Geen toegang: deze pagina heeft geen geldige sleutel.",
        ("unauthorized", true) => "No access: this page doesn\u{2019}t have a valid key.",
        ("badResponse", false) => "Onverwacht antwoord van Win11 Shell.",
        ("badResponse", true) => "Unexpected response from Win11 Shell.",
        (_, false) => "Er ging iets mis.",
        (_, true) => "Something went wrong.",
    }
}

/// How the app starts a backend (`win11-app --app <kind> --http-only` next to the app).
struct Spawner {
    host: PathBuf,
    kind: String,
    instance: PathBuf,
    dry: bool,
}

impl Spawner {
    fn spawn(&self) -> Result<Child, String> {
        // a file of an earlier backend must not be taken for this one's
        let _ = std::fs::remove_file(&self.instance);
        let _ = std::fs::remove_file(restart_note_path(&self.instance));
        let mut command = Command::new(&self.host);
        command
            .args(["--app", &self.kind, "--http-only"])
            .env("WIN11_NATIVE_INSTANCE", &self.instance)
            .env_remove("WIN11_DEV_COMMAND")
            .env_remove("WIN11_DEV_REPORT");
        // `win11-app --app <kind> --dry-run` execs the app with --dry-run (host lib.rs exec_native_app): the backend
        // is the one that acts, so it must get the flag too (WIN11SHELL_DRY_RUN reaches it through the environment).
        if self.dry {
            command.arg("--dry-run");
        }
        let parent = std::process::id();
        // SAFETY: pre_exec runs in the forked child before exec; prctl and getppid are async-signal-safe.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(move || {
                // The backend lives exactly as long as the thread that starts it: the main thread for the first
                // one, the supervisor (alive as long as the app) for every later one.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if libc::getppid() as u32 != parent {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
        command.spawn().map_err(|e| format!("{}: {e}", self.host.display()))
    }

    /// Wait until the child wrote its instance file (at most 30 s).
    fn wait_ready(&self, child: &mut Child) -> Result<Endpoint, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some((port, token)) = read_instance(&self.instance, child.id()) {
                return Ok((port, Arc::new(token)));
            }
            if let Ok(Some(status)) = child.try_wait() {
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

    /// The restart note of the backend `pid`, taken (the file goes).
    fn take_note(&self, pid: u32) -> Option<RestartNote> {
        let path = restart_note_path(&self.instance);
        let bytes = std::fs::read(&path).ok();
        let _ = std::fs::remove_file(&path);
        RestartNote::parse(&serde_json::from_slice(&bytes?).ok()?, pid)
    }
}

impl Backend {
    /// A client that is never ready (no backend: every call fails with "offline").
    pub fn none(reason: &str) -> Backend {
        let b = Backend::empty();
        b.set_ready(Err(reason.to_owned()));
        b
    }

    fn empty() -> Backend {
        Backend {
            ready: Arc::default(),
            lang: Arc::new(Mutex::new("nl".into())),
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            life: Arc::default(),
        }
    }

    /// Start `win11-app --app <kind> --http-only` (or take WIN11_APP_BACKEND) and return at once; `on_ready`
    /// runs on a helper thread once it answers (a later backend sends "backend.ready" {respawn: true}).
    pub fn start(kind: &str, on_ready: impl FnOnce() + Send + 'static) -> Result<Backend, String> {
        let backend = Backend::empty();
        if let Ok(spec) = std::env::var("WIN11_APP_BACKEND") {
            let (port, token) = spec.split_once(':').ok_or("WIN11_APP_BACKEND is port:token")?;
            let port: u16 = port.trim().parse().map_err(|_| "WIN11_APP_BACKEND: bad port")?;
            backend.set_ready(Ok((port, Arc::new(token.trim().to_owned()))));
            on_ready();
            return Ok(backend);
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let host = exe.with_file_name("win11-app");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = runtime.join("win11-gnome");
        // Private (0700), as the host demands of it (host events.rs private_dir): a folder made here with the umask's
        // mode made every backend refuse to start ("De runtime-map moet privé zijn").
        let _ = std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(&dir);
        let spawner = Spawner {
            host,
            kind: kind.to_owned(),
            instance: dir.join(format!("native-{kind}-{}.json", std::process::id())),
            dry: std::env::args().any(|arg| arg == "--dry-run"),
        };
        // the first one here: a backend that cannot be started at all is an error of the app's start
        let child = spawner.spawn()?;
        *lock(&backend.life.instance) = Some(spawner.instance.clone());
        let supervisor = backend.clone();
        std::thread::Builder::new()
            .name("win11-kit-backend".into())
            .spawn(move || supervisor.supervise(spawner, child, Box::new(on_ready)))
            .map_err(|e| e.to_string())?;
        Ok(backend)
    }

    /// The supervisor thread: wait for each backend, then restart the app or start a new backend.
    fn supervise(&self, spawner: Spawner, first: Child, on_ready: Box<dyn FnOnce() + Send>) {
        let mut on_ready = Some(on_ready);
        let mut next = Some(first);
        let mut failures = 0u32;
        loop {
            let mut child = match next.take().map(Ok).unwrap_or_else(|| spawner.spawn()) {
                Ok(child) => child,
                Err(error) => {
                    eprintln!("win11-kit: backend: {error}");
                    if !self.wait_before_respawn(&mut failures) {
                        return;
                    }
                    continue;
                }
            };
            let pid = child.id();
            match spawner.wait_ready(&mut child) {
                Ok(endpoint) => {
                    let up = Instant::now();
                    self.set_ready(Ok(endpoint));
                    match on_ready.take() {
                        Some(ready) => ready(),
                        None => crate::ctx::dispatch("backend.ready".into(), json!({ "respawn": true })),
                    }
                    let status = child.wait();
                    // calls wait for the next backend from here on
                    *lock(&self.ready.endpoint) = None;
                    let _ = std::fs::remove_file(&spawner.instance);
                    if let Some(note) = spawner.take_note(pid) {
                        if !self.restart(note) {
                            return;
                        }
                        failures = 0;
                        continue;
                    }
                    let status = match status {
                        Ok(s) if s.code() == Some(RESTART_EXIT) => format!("{s}, a restart without its note"),
                        Ok(s) => s.to_string(),
                        Err(e) => e.to_string(),
                    };
                    eprintln!("win11-kit: the backend ended ({status}); starting a new one");
                    if up.elapsed() >= STABLE {
                        failures = 0;
                    }
                }
                Err(error) => eprintln!("win11-kit: backend: {error}"),
            }
            if !self.wait_before_respawn(&mut failures) {
                return;
            }
        }
    }

    /// Count a failed backend; false (offline for good) after MAX_RESPAWNS in a row, else wait its delay.
    fn wait_before_respawn(&self, failures: &mut u32) -> bool {
        *failures += 1;
        if *failures > MAX_RESPAWNS {
            eprintln!("win11-kit: the backend failed {MAX_RESPAWNS} times in a row; staying offline");
            self.set_ready(Err("the backend keeps failing".into()));
            return false;
        }
        std::thread::sleep(respawn_delay(*failures));
        true
    }

    /// A backend restarted: the main thread decides ("backend.restart", kit app.rs). True: start a new backend.
    fn restart(&self, note: RestartNote) -> bool {
        let data = json!({ "all": note.all, "origin": note.origin });
        *lock(&self.life.note) = Some(note);
        *lock(&self.life.answer) = None;
        crate::ctx::dispatch("backend.restart".into(), data);
        let mut answer = lock(&self.life.answer);
        let deadline = Instant::now() + Duration::from_secs(30);
        while answer.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                // nobody answered (no context): keep the app online
                return true;
            }
            answer = self.life.answered.wait_timeout(answer, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        answer.take().unwrap_or(true)
    }

    /// The note of the backend that restarted (the main thread, on "backend.restart"); taken once.
    pub(crate) fn take_restart(&self) -> Option<RestartNote> {
        lock(&self.life.note).take()
    }

    /// The answer to "backend.restart": `respawn` true starts a new backend; false: the app execs itself or quits.
    pub(crate) fn answer_restart(&self, respawn: bool) {
        *lock(&self.life.answer) = Some(respawn);
        self.life.answered.notify_all();
    }

    /// The app ends: the instance file of its backend goes (the backend itself ends through PR_SET_PDEATHSIG).
    pub(crate) fn close(&self) {
        if let Some(path) = lock(&self.life.instance).take() {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn set_ready(&self, endpoint: Result<Endpoint, String>) {
        *lock(&self.ready.endpoint) = Some(endpoint);
        self.ready.changed.notify_all();
    }

    /// Whether the backend has answered (it may still be starting).
    pub fn is_ready(&self) -> bool {
        matches!(&*lock(&self.ready.endpoint), Some(Ok(_)))
    }

    /// Whether the event stream is connected now.
    pub fn connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn en(&self) -> bool {
        *lock(&self.lang) == "en"
    }

    /// The backend's port and token, waiting (off the main thread) until it is up.
    fn endpoint(&self) -> Result<Endpoint, CallError> {
        let mut guard = lock(&self.ready.endpoint);
        let deadline = Instant::now() + Duration::from_secs(35);
        while guard.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            guard = self.ready.changed.wait_timeout(guard, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        match &*guard {
            Some(Ok(ep)) => Ok(ep.clone()),
            _ => Err(CallError::new("offline", text("offline", self.en()))),
        }
    }

    /// The X-Shell-Lang the host answers its texts in ("nl" / "en"; the kit keeps it on settings.language).
    pub fn set_lang(&self, lang: &str) {
        *lock(&self.lang) = lang.to_owned();
    }

    /// One API call, blocking: use it from worker threads (the main thread uses `Ctx::call`).
    pub fn call(&self, method: &str, args: &Value) -> Result<Value, CallError> {
        let (port, token) = self.endpoint()?;
        let args = if args.is_null() { json!({}) } else { args.clone() };
        let body = json!({ "args": args }).to_string();
        let lang = lock(&self.lang).clone();
        let request = format!(
            "POST /api/call/{method} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nX-Shell-Token: {token}\r\nX-Shell-Lang: {lang}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let en = lang == "en";
        let (status, data) =
            self.exchange(port, request.as_bytes()).map_err(|_| CallError::new("offline", text("offline", en)))?;
        if status == 401 || status == 403 {
            return Err(CallError::new("unauthorized", text("unauthorized", en)));
        }
        let v: Value =
            serde_json::from_slice(&data).map_err(|_| CallError::new("bad_response", text("badResponse", en)))?;
        if v["ok"] == true {
            return Ok(v["result"].clone());
        }
        Err(CallError::from_json(&v["error"], text("failed", en)))
    }

    /// GET a backend path, blocking ("/icon?name=firefox&size=32", "/thumb?path=..&size=256"); the token is
    /// added. Off the main thread, or through `Ctx::get`.
    pub fn get(&self, path_and_query: &str) -> Result<Vec<u8>, CallError> {
        let (port, token) = self.endpoint()?;
        let sep = if path_and_query.contains('?') { '&' } else { '?' };
        let request = format!(
            "GET {path_and_query}{sep}token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let (status, data) =
            self.exchange(port, request.as_bytes()).map_err(|_| CallError::new("offline", text("offline", self.en())))?;
        if status == 200 { Ok(data) } else { Err(CallError::new("http", format!("HTTP {status}"))) }
    }

    /// Whether the backend at `port` is still the one to talk to: no other backend took its place, and it
    /// accepts a connection.
    fn still_there(&self, port: u16) -> bool {
        let same = match &*lock(&self.ready.endpoint) {
            Some(Ok((current, _))) => *current == port,
            Some(Err(_)) => false,
            // between two backends: the old one is gone
            None => false,
        };
        same && TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(3)).is_ok()
    }

    fn exchange(&self, port: u16, request: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_read_timeout(Some(QUIET)).ok();
        stream.write_all(request).map_err(|e| e.to_string())?;
        let raw = read_until_closed(&mut stream, || self.still_there(port))?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("bad HTTP response")?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut body = raw[split + 4..].to_vec();
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            body = dechunk(&body);
        }
        Ok((status, body))
    }

    /// The event stream: `on(name, data)` from a reader thread, reconnecting after errors; `status(connected)`
    /// whenever the stream connects or drops.
    pub fn events(&self, on: fn(String, Value), status: fn(bool)) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("win11-kit-events".into())
            .spawn(move || {
                let mut wait = 1000u64;
                loop {
                    let opened = me.read_events(on, status);
                    if me.connected.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        status(false);
                    }
                    if opened {
                        // the stream was up: its backend ended (a restart, a new backend); back soon
                        wait = 300;
                    }
                    if matches!(&*lock(&me.ready.endpoint), Some(Err(_))) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(wait));
                    wait = (wait * 2).clamp(1000, 15_000);
                }
            })
            .ok();
    }
    /// Read the stream until it ends; true when it was open for a while.
    fn read_events(&self, on: fn(String, Value), status: fn(bool)) -> bool {
        let Ok((port, token)) = self.endpoint() else { return false };
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else { return false };
        let request = format!(
            "GET /api/events?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\n\r\n"
        );
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return false,
                Ok(_) => {}
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        self.connected.store(true, std::sync::atomic::Ordering::Relaxed);
        status(true);
        let (mut event, mut data) = (String::new(), String::new());
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return true,
                Ok(_) => {}
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
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.trim_start());
            }
        }
    }
}

/// Read a response until the backend closes the connection. A read that waits [`QUIET`] asks `alive()` whether
/// to wait on (a long call of a live backend) or to give up (the backend is gone).
pub(crate) fn read_until_closed(stream: &mut impl Read, alive: impl Fn() -> bool) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok(raw),
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if !alive() {
                    return Err("the backend is gone".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
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

/// Percent-encode a query value (icon names, paths).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("win11-kit-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_restart_note_and_an_instance_file_belong_to_their_backend() {
        let note = json!({"version":1,"pid":42,"all":true,"origin":true,"token":"tok"});
        assert_eq!(
            RestartNote::parse(&note, 42),
            Some(RestartNote { all: true, origin: true, token: Some("tok".into()) })
        );
        assert_eq!(RestartNote::parse(&note, 43), None, "another backend's note");
        let plain = json!({"pid":42,"all":false,"origin":false,"token":null});
        assert_eq!(RestartNote::parse(&plain, 42), Some(RestartNote::default()));
        let bad = json!({"pid":42,"all":true,"token":"a\u{0}b"});
        assert_eq!(RestartNote::parse(&bad, 42).and_then(|n| n.token), None, "no NUL in a token");
        assert_eq!(
            restart_note_path(Path::new("/r/win11-gnome/native-explorer-7.json")),
            PathBuf::from("/r/win11-gnome/native-explorer-7.json.restart")
        );
        let dir = scratch("instance");
        let file = dir.join("native-x-1.json");
        std::fs::write(&file, r#"{"port":4242,"token":"t","pid":9}"#).unwrap();
        assert_eq!(read_instance(&file, 9), Some((4242, "t".into())));
        assert_eq!(read_instance(&file, 10), None, "an earlier backend's file");
        std::fs::write(&file, r#"{"port":0,"token":"t","pid":9}"#).unwrap();
        assert_eq!(read_instance(&file, 9), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_host_error_keeps_its_catalogue_key() {
        // host files/mod.rs ApiError::json: {code, message, key?, params?, data?}
        let e = CallError::from_json(
            &json!({"code":"not_ready","message":"Bluetooth reageert niet.","key":"system.bt_not_responding","params":{}}),
            "x",
        );
        assert_eq!((e.code.as_str(), e.message.as_str(), e.key.as_deref()), ("not_ready", "Bluetooth reageert niet.", Some("system.bt_not_responding")));
        assert_eq!(e.data, Value::Null);
        let e = CallError::from_json(&json!({"code":"untrusted","message":"m","data":{"path":"/p"}}), "x");
        assert_eq!((e.key, e.data), (None, json!({"path":"/p"})));
        let e = CallError::from_json(&Value::Null, "Er ging iets mis.");
        assert_eq!((e.code.as_str(), e.message.as_str(), e.key), ("internal", "Er ging iets mis.", None));
        assert_eq!(CallError::from_json(&json!({"code":"c","message":"m","key":""}), "x").key, None);
    }

    #[test]
    fn new_backends_back_off() {
        let delays: Vec<u64> = (1..=MAX_RESPAWNS + 2).map(|n| respawn_delay(n).as_millis() as u64).collect();
        assert_eq!(delays, [500, 1000, 2000, 4000, 8000, 8000, 8000]);
    }

    /// A stream that has nothing to say for `quiet` reads (a long call), then answers and closes.
    struct Slow {
        quiet: usize,
        answer: &'static [u8],
    }
    impl Read for Slow {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.quiet > 0 {
                self.quiet -= 1;
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let n = self.answer.len().min(buf.len());
            buf[..n].copy_from_slice(&self.answer[..n]);
            self.answer = &self.answer[n..];
            Ok(n)
        }
    }

    #[test]
    fn a_long_call_waits_while_its_backend_lives_and_ends_when_it_is_gone() {
        let checks = std::cell::Cell::new(0);
        let mut slow = Slow { quiet: 8, answer: b"HTTP/1.1 200 OK\r\n\r\n{}" };
        let raw = read_until_closed(&mut slow, || {
            checks.set(checks.get() + 1);
            true
        })
        .unwrap();
        assert_eq!(raw, b"HTTP/1.1 200 OK\r\n\r\n{}");
        assert_eq!(checks.get(), 8, "asked after every quiet read, never gave up");
        let mut gone = Slow { quiet: 3, answer: b"" };
        let asked = std::cell::Cell::new(0);
        assert!(read_until_closed(&mut gone, || {
            asked.set(asked.get() + 1);
            asked.get() < 2
        })
        .is_err());
        assert_eq!(asked.get(), 2, "gives up once the backend is gone");
    }

    /// A stand-in for `win11-app --app <kind> --http-only`: run n writes its instance file, then
    /// 1: crashes (a new backend), 2: a plain shell.restart (a new backend), 3+: shell.restartAll.
    const FAKE: &str = r#"#!/bin/sh
n=$(cat "$WIN11_NATIVE_INSTANCE.n" 2>/dev/null || echo 0)
n=$((n+1))
echo $n > "$WIN11_NATIVE_INSTANCE.n"
printf '{"port":%d,"token":"t%d","pid":%d}' $((40000+n)) $n $$ > "$WIN11_NATIVE_INSTANCE.tmp"
mv "$WIN11_NATIVE_INSTANCE.tmp" "$WIN11_NATIVE_INSTANCE"
sleep 0.3
case $n in
1) exit 3 ;;
2) printf '{"version":1,"pid":%d,"all":false,"origin":false,"token":null}' $$ > "$WIN11_NATIVE_INSTANCE.restart"; exit 75 ;;
*) printf '{"version":1,"pid":%d,"all":true,"origin":true,"token":"tok"}' $$ > "$WIN11_NATIVE_INSTANCE.restart"; exit 75 ;;
esac
"#;

    fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn port(b: &Backend) -> Option<u16> {
        match &*lock(&b.ready.endpoint) {
            Some(Ok((port, _))) => Some(*port),
            _ => None,
        }
    }

    #[test]
    fn the_supervisor_starts_a_new_backend_after_a_crash_and_hands_a_restart_to_the_app() {
        let dir = scratch("supervisor");
        let host = dir.join("win11-app");
        std::fs::write(&host, FAKE).unwrap();
        std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
        let spawner =
            Spawner { host, kind: "explorer".into(), instance: dir.join("native-explorer-1.json"), dry: false };
        let backend = Backend::empty();
        let child = spawner.spawn().unwrap();
        let (tx, ready) = std::sync::mpsc::channel();
        let supervisor = backend.clone();
        let thread = std::thread::spawn(move || {
            supervisor.supervise(spawner, child, Box::new(move || tx.send(()).unwrap()))
        });
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(port(&backend), Some(40001), "the first backend");
        // it crashes: no restart for the app, a new backend after 0.5 s
        assert_eq!(until("the second backend", || port(&backend).filter(|p| *p == 40002)), 40002);
        assert_eq!(backend.take_restart(), None);
        // a plain shell.restart: the app is asked; it stays, so a third backend comes at once
        let note = until("the plain restart", || backend.take_restart());
        assert_eq!(note, RestartNote::default());
        assert!(!backend.is_ready(), "calls wait between two backends");
        backend.answer_restart(true);
        assert_eq!(until("the third backend", || port(&backend).filter(|p| *p == 40003)), 40003);
        // shell.restartAll: the app execs itself (here: says so), and the supervisor ends
        let note = until("restartAll", || backend.take_restart());
        assert_eq!(note, RestartNote { all: true, origin: true, token: Some("tok".into()) });
        backend.answer_restart(false);
        thread.join().unwrap();
        assert!(!backend.is_ready());
        assert!(!restart_note_path(&dir.join("native-explorer-1.json")).exists(), "the note was taken");
        assert!(!dir.join("native-explorer-1.json").exists(), "no instance file of an ended backend");
        let _ = std::fs::remove_dir_all(dir);
    }
}
