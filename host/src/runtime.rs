//! Authenticated native HTTP/API runtime of a host process: the backend of win11-shell and of each native app
//! (lib.rs run_http_only), and the file portal's own (portal.rs).
use crate::{
    apps,
    backend::{self, Backend},
    events, files, gnome, images, services, settings_runtime, transfers,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub(crate) struct Runtime {
    pub backend: Backend,
    pub ctx: Arc<services::ServiceContext>,
    pub files: Arc<files::Files>,
    settings: Option<Arc<settings_runtime::SettingsRuntime>>,
    apps: Option<Arc<apps::Apps>>,
    services: Option<Arc<services::NativeServices>>,
    notifications: Arc<services::notifications::Notifications>,
    core: Option<Arc<services::shell::NativeCore>>,
    pub transfers: Arc<transfers::Transfers>,
    ui: PathBuf,
    kind: &'static str,
    /// The last beat of the owner's GLib main loop (a timer in every owner: lib.rs `run_http_only`, portal.rs);
    /// `shell.info` reports its age as `mainLoopLagMs`.
    beat: Mutex<Option<Instant>>,
}
impl Runtime {
    pub fn new(
        ui: PathBuf,
        kind: &'static str,
        shutdown: Arc<dyn Fn(bool) + Send + Sync>,
    ) -> Result<Arc<Self>, String> {
        let backend =
            backend::BackendGuard::connect(ui.parent().ok_or("UI-map ontbreekt")?, kind)?.backend;
        let files = files::Files::new(&backend)?;
        let settings =
            settings_runtime::SettingsRuntime::new(&backend, files.events.clone(), &ui, kind)?;
        let apps = apps::Apps::new(files.clone())?;
        let config = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
        let ctx = services::ServiceContext::new(
            files.events.clone(),
            config,
            glib::user_cache_dir().join("win11-gnome"),
            ui.clone(),
            kind.into(),
            backend::dry_run(),
        )?;
        let notifications = services::notifications::Notifications::new(ctx.clone())?;
        let native = backend.port == 0;
        let core = if native {
            Some(services::shell::NativeCore::new(ctx.clone(), shutdown)?)
        } else {
            None
        };
        let services = if native {
            Some(services::NativeServices::new(
                ctx.clone(),
                notifications.clone(),
            )?)
        } else {
            None
        };
        let runtime = Arc::new(Self {
            backend,
            ctx: ctx.clone(),
            files,
            settings,
            apps,
            services,
            notifications,
            core,
            transfers: Arc::new(transfers::Transfers::new()?),
            ui,
            kind,
            beat: Mutex::new(Some(Instant::now())),
        });
        let weak = Arc::downgrade(&runtime);
        ctx.set_caller(Arc::new(move |method, args| {
            weak.upgrade()
                .ok_or_else(|| {
                    files::ApiError::tr(
                        "offline",
                        crate::i18n::current_language(),
                        "core.serviceStopped",
                        crate::i18n::NO_PARAMS,
                    )
                })?
                .call(method, args)
                .unwrap_or_else(|| Err(unknown(crate::i18n::current_language(), method)))
        }));
        Ok(runtime)
    }
    pub fn shutdown(&self) {
        if let Some(core) = &self.core {
            core.shutdown();
        }
        if let Some(svc) = &self.services {
            svc.shutdown();
        }
        self.notifications.shutdown();
        if let Some(apps) = &self.apps {
            apps.shutdown();
        }
        if let Some(settings) = &self.settings {
            settings.shutdown();
        }
        self.files.shutdown();
    }
    pub fn set_runtime_info(&self, data: Value) {
        if let Some(core) = &self.core {
            core.set_runtime_info(data);
        }
    }
    /// D-G74a: make this host the owner of WinPoint's update check (lib.rs run_http_only, on its main thread).
    pub fn winpoint_start(&self) {
        if let Some(services) = &self.services {
            services.winpoint_start();
        }
    }
    pub fn beat(&self) {
        *self.beat.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    pub fn call(&self, method: &str, args: &Value) -> Option<files::ApiResult> {
        let empty = json!({});
        let args = if args.is_null() { &empty } else { args };
        if is_known(method) && !args.is_object() {
            return Some(Err(files::ApiError::tr(
                "bad_args",
                self.ctx.lang(),
                "core.badArgs",
                crate::i18n::NO_PARAMS,
            )));
        }
        if method.starts_with("explorer.tab") && is_known(method) {
            return Some(
                self.transfers
                    .call(method, args)
                    .map_err(|e| files::ApiError::new("tab_transfer", &e)),
            );
        }
        if method.starts_with("windows.") && is_known(method) {
            if self.ctx.dry_run && !matches!(method, "windows.list" | "windows.setIconRects") {
                return Some(Ok(json!({"ok":true,"dryRun":true})));
            }
            return Some(
                gnome::call_api(method, args)
                    .unwrap_or_else(|e| Err(files::ApiError::new("gnome", &e))),
            );
        }
        if let Some(mut result) = self.core.as_ref().and_then(|s| s.call(method, args)) {
            if method == "shell.info" {
                if let Ok(value) = &mut result {
                    if let Some(beat) = *self.beat.lock().unwrap_or_else(|e| e.into_inner()) {
                        value["mainLoopLagMs"] = json!(beat.elapsed().as_millis() as u64);
                    }
                }
            }
            return Some(result);
        }
        if let Some(result) = self.settings.as_ref().and_then(|s| s.call(method, args)) {
            return Some(result);
        }
        if let Some(result) = self.apps.as_ref().and_then(|s| s.call(method, args)) {
            return Some(result);
        }
        if let Some(result) = self.files.call(method, args) {
            return Some(result);
        }
        if let Some(result) = self.services.as_ref().and_then(|s| s.call(method, args)) {
            return Some(result);
        }
        self.notifications.call(method, args)
    }
    pub fn server(self: &Arc<Self>) -> Result<(u16, Arc<AtomicBool>), String> {
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = server.server_addr().to_ip().ok_or("Geen TCP-adres")?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let runtime = self.clone();
        std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(200)) {
                    Ok(Some(req)) => {
                        let rt = runtime.clone();
                        std::thread::spawn(move || rt.serve(req, port));
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });
        Ok((port, stop))
    }
    fn serve(self: Arc<Self>, mut request: tiny_http::Request, port: u16) {
        let parsed = match url::Url::parse(&format!("http://localhost{}", request.url())) {
            Ok(v) => v,
            Err(_) => return text(request, 400, "Bad URL"),
        };
        let path = parsed.path();
        // The UI's assets, without a token: the built-in wallpapers and the Settings icon, which win11-shell and
        // the native apps fetch (native/shell desktop/wallpaper.rs, panel/taskbar/util.rs; native/apps/settings
        // common.rs). Nothing else of the UI folder is served.
        if let Some(relative) = path.strip_prefix("/ui/") {
            if !matches!(
                request.method(),
                tiny_http::Method::Get | tiny_http::Method::Head
            ) {
                return text(request, 405, "Method Not Allowed");
            }
            let assets = self.ui.join("assets");
            let target = relative
                .strip_prefix("assets/")
                .and_then(|relative| url::Url::parse(&format!("file:///{relative}")).ok())
                .and_then(|u| u.to_file_path().ok())
                .map(|p| assets.join(p.strip_prefix("/").unwrap_or(&p)));
            if let Some(target) = target.and_then(|p| p.canonicalize().ok()).filter(|p| {
                assets.canonicalize().is_ok_and(|root| p.starts_with(root)) && p.is_file()
            }) {
                return match std::fs::read(&target) {
                    Ok(data) => bytes(request, 200, data, crate::content_type(&target)),
                    Err(_) => text(request, 404, "Not Found"),
                };
            }
            return text(request, 404, "Not Found");
        }
        let query_token = parsed
            .query_pairs()
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned());
        let supplied = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("X-Shell-Token"))
            .map(|h| h.value.as_str());
        if !constant_equal(
            supplied.or(query_token.as_deref()).unwrap_or(""),
            &self.backend.token,
        ) {
            return text(request, 401, "Unauthorized");
        }
        if request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Origin"))
            .is_some_and(|h| {
                h.value.as_str() != format!("http://127.0.0.1:{port}")
                    && h.value.as_str() != format!("http://localhost:{port}")
            })
        {
            return json_response(
                request,
                403,
                failure(files::ApiError::tr(
                    "forbidden",
                    self.ctx.lang(),
                    "core.forbidden",
                    crate::i18n::NO_PARAMS,
                )),
            );
        }
        if path == "/api/events" {
            if request.method() != &tiny_http::Method::Get {
                return text(request, 405, "Method Not Allowed");
            }
            return self.events(request);
        }
        if matches!(path, "/icon" | "/thumb" | "/file") {
            if !matches!(
                request.method(),
                tiny_http::Method::Get | tiny_http::Method::Head
            ) {
                return text(request, 405, "Method Not Allowed");
            }
            if let Some(response) = images::handle(path, &parsed, request.headers(), &self.ctx) {
                return crate::respond_image(request, response);
            }
        }
        if path == "/api/methods" {
            if request.method() != &tiny_http::Method::Get {
                return text(request, 405, "Method Not Allowed");
            }
            if self.backend.port == 0 {
                return json_response(request, 200, json!({"ok":true,"result":methods()}));
            }
        }
        if let Some(encoded) = path.strip_prefix("/api/call/") {
            if request.method() != &tiny_http::Method::Post {
                return json_response(
                    request,
                    405,
                    failure(files::ApiError::tr(
                        "method_not_allowed",
                        self.ctx.lang(),
                        "core.methodNotAllowed",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            }
            let method = decode_method(encoded);
            let Some(method) = method.filter(|s| {
                !s.is_empty()
                    && s.len() <= 100
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            }) else {
                return json_response(
                    request,
                    400,
                    failure(files::ApiError::tr(
                        "bad_request",
                        self.ctx.lang(),
                        "core.badRequest",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            };
            if request.headers().iter().any(|h| {
                h.field.equiv("Transfer-Encoding")
                    && h.value.as_str().to_lowercase().contains("chunked")
            }) {
                return json_response(
                    request,
                    411,
                    failure(files::ApiError::tr(
                        "length_required",
                        self.ctx.lang(),
                        "core.lengthRequired",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            }
            if request.body_length().is_some_and(|n| n > 16 * 1024 * 1024) {
                return json_response(
                    request,
                    413,
                    failure(files::ApiError::tr(
                        "too_large",
                        self.ctx.lang(),
                        "core.tooLarge",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            }
            let mut data = vec![];
            if request
                .as_reader()
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut data)
                .is_err()
                || data.len() > 16 * 1024 * 1024
            {
                return json_response(
                    request,
                    413,
                    failure(files::ApiError::tr(
                        "too_large",
                        self.ctx.lang(),
                        "core.tooLarge",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            }
            let payload = if data.iter().all(u8::is_ascii_whitespace) {
                Some(json!({}))
            } else {
                serde_json::from_slice::<Value>(&data)
                    .ok()
                    .filter(Value::is_object)
            };
            let Some(payload) = payload else {
                return json_response(
                    request,
                    200,
                    failure(files::ApiError::tr(
                        "bad_request",
                        self.ctx.lang(),
                        "core.badRequest",
                        crate::i18n::NO_PARAMS,
                    )),
                );
            };
            if let Some(result) = self.call(&method, &payload["args"]) {
                return json_response(request, 200, envelope(result, &method, self.ctx.dry_run));
            }
            if self.backend.port == 0 {
                return json_response(
                    request,
                    200,
                    envelope(Err(unknown(self.ctx.lang(), &method)), &method, false),
                );
            }
            return self.proxy(request, &data);
        }
        if self.backend.port > 0 {
            return self.proxy(request, &[]);
        }
        if request.method() == &tiny_http::Method::Post {
            text(request, 405, "Method Not Allowed")
        } else {
            text(request, 404, "Not Found")
        }
    }
    fn proxy(&self, request: tiny_http::Request, data: &[u8]) {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(40))
            .build();
        let call = agent
            .request(
                request.method().as_str(),
                &format!("http://127.0.0.1:{}{}", self.backend.port, request.url()),
            )
            .set("X-Shell-Token", &self.backend.token)
            .set("Content-Type", "application/json");
        let reply = if request.method() == &tiny_http::Method::Get {
            call.call()
        } else {
            call.send_bytes(data)
        };
        match reply {
            Ok(r) | Err(ureq::Error::Status(_, r)) => {
                let status = r.status();
                let mime = r
                    .header("Content-Type")
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let length = r.header("Content-Length").and_then(|s| s.parse().ok());
                let _ = request.respond(tiny_http::Response::new(
                    tiny_http::StatusCode(status),
                    headers(&mime),
                    r.into_reader(),
                    length,
                    None,
                ));
            }
            Err(e) => json_response(
                request,
                502,
                failure(files::ApiError::tr(
                    "offline",
                    self.ctx.lang(),
                    "core.referenceOffline",
                    &serde_json::json!({"error": e.to_string()}),
                )),
            ),
        }
    }
    fn events(&self, request: tiny_http::Request) {
        let stream = self.files.events.subscribe();
        let mut hello_frame = None;
        if self.backend.port > 0 {
            let upstream = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(3))
                .timeout_read(Duration::from_secs(40))
                .build()
                .get(&format!(
                    "http://127.0.0.1:{}/api/events?token={}",
                    self.backend.port, self.backend.token
                ))
                .call();
            let Ok(upstream) = upstream else {
                return text(
                    request,
                    502,
                    &crate::i18n::tr(self.ctx.lang(), "core.referenceUnreachable"),
                );
            };
            let sender = stream.sender.clone();
            let closed = stream.closed.clone();
            std::thread::spawn(move || {
                let mut input = BufReader::new(upstream.into_reader());
                let mut frame = vec![];
                let mut line = vec![];
                while !closed.load(Ordering::Relaxed) {
                    line.clear();
                    match input.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            frame.extend(&line);
                            if frame.len() > 4 * 1024 * 1024 {
                                break;
                            }
                            if line == b"\n" || line == b"\r\n" {
                                if frame.starts_with(b"event: notifications.") {
                                    frame.clear();
                                    continue;
                                }
                                if sender.try_send(std::mem::take(&mut frame)).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                let _ = sender.try_send(vec![]);
                closed.store(true, Ordering::Relaxed);
            });
        } else {
            let hello = json!({"mode":if self.kind=="panel"{"layer"}else{"app"},"version":crate::VERSION,"dryRun":self.ctx.dry_run,"compositor":"gnome"});
            hello_frame = Some(format!("retry: 1500\n\nevent: hello\ndata: {hello}\n\n"));
        }
        let mut writer = request.into_writer();
        if writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n").and_then(|_|writer.flush()).is_err(){return;}
        if let Some(hello) = hello_frame {
            if writer
                .write_all(hello.as_bytes())
                .and_then(|_| writer.flush())
                .is_err()
            {
                return;
            }
        }
        let mut stream = stream;
        let mut buffer = [0u8; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer
                        .write_all(&buffer[..n])
                        .and_then(|_| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }
}
fn constant_equal(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (i, value) in b.bytes().enumerate() {
        diff |= (a.as_bytes().get(i).copied().unwrap_or(0) ^ value) as usize;
    }
    diff == 0
}
fn unknown(lang: crate::i18n::Lang, method: &str) -> files::ApiError {
    let mut e = files::ApiError::tr(
        "unknown_method",
        lang,
        "core.unknownMethod",
        crate::i18n::NO_PARAMS,
    );
    e.data = Some(json!({"method":method}));
    e
}
/// The method list (`/api/methods`): api-methods.json plus one fragment per Settings batch
/// (notes/claude/settings-parity.md §3). Each entry is `{name, dangerous, doc}`, optionally `secret` (amendment 53).
const METHOD_FILES: &[(&str, &str)] = &[
    ("api-methods.json", include_str!("api-methods.json")),
    (
        "api-methods/foundation.json",
        include_str!("api-methods/foundation.json"),
    ),
    (
        "api-methods/input.json",
        include_str!("api-methods/input.json"),
    ),
    (
        "api-methods/devices.json",
        include_str!("api-methods/devices.json"),
    ),
    (
        "api-methods/network.json",
        include_str!("api-methods/network.json"),
    ),
    (
        "api-methods/system.json",
        include_str!("api-methods/system.json"),
    ),
    (
        "api-methods/accounts.json",
        include_str!("api-methods/accounts.json"),
    ),
    (
        "api-methods/apps.json",
        include_str!("api-methods/apps.json"),
    ),
];
/// The sorted union of the method files; a malformed entry or a name in two places is an error (the build's tests
/// and the first call catch it).
pub(crate) fn load_methods(files: &[(&str, &str)]) -> Result<Vec<Value>, String> {
    let mut all: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashMap::new();
    for (file, text) in files {
        let list: Vec<Value> = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
        for entry in list {
            let name = entry["name"]
                .as_str()
                .filter(|n| !n.is_empty() && n.len() <= 100)
                .ok_or_else(|| format!("{file}: an entry without a name"))?
                .to_owned();
            let object = entry.as_object().ok_or_else(|| format!("{file}: {name}"))?;
            if !entry["dangerous"].is_boolean()
                || !entry["doc"].is_string()
                || !entry["secret"].is_null() && !entry["secret"].is_boolean()
                || object
                    .keys()
                    .any(|k| !matches!(k.as_str(), "name" | "dangerous" | "doc" | "secret"))
            {
                return Err(format!(
                    "{file}: {name} is not {{name, dangerous, doc, secret?}}"
                ));
            }
            if let Some(other) = seen.insert(name.clone(), *file) {
                return Err(format!("{name} is listed in {other} and {file}"));
            }
            all.push(entry);
        }
    }
    all.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(all)
}
pub(crate) fn methods() -> &'static Vec<Value> {
    static METHODS: OnceLock<Vec<Value>> = OnceLock::new();
    METHODS.get_or_init(|| load_methods(METHOD_FILES).expect("embedded API methods"))
}
/// A method whose arguments carry a password, PIN or key (amendment 53): its args, error data and dry-run toasts are
/// never logged.
#[allow(dead_code)] // the batches' logging paths and the harness redaction (step I)
pub(crate) fn is_secret(method: &str) -> bool {
    methods()
        .iter()
        .any(|m| m["name"] == method && m["secret"] == true)
}
fn is_known(method: &str) -> bool {
    methods().iter().any(|m| m["name"] == method)
}
fn envelope(result: files::ApiResult, method: &str, dry: bool) -> Value {
    match result {
        Ok(v) => {
            let mut out = json!({"ok":true,"result":v});
            if dry
                && methods()
                    .iter()
                    .any(|m| m["name"] == method && m["dangerous"] == true)
            {
                out["dryRun"] = json!(true);
            }
            out
        }
        Err(e) => json!({"ok":false,"error":e.json()}),
    }
}
/// An error envelope outside a method call (contracts C2: code, message, key, params).
fn failure(error: files::ApiError) -> Value {
    json!({"ok":false,"error":error.json()})
}
fn headers(mime: &str) -> Vec<tiny_http::Header> {
    vec![
        crate::header("Content-Type", mime),
        crate::header("Cache-Control", "no-store"),
        crate::header("X-Content-Type-Options", "nosniff"),
        crate::header("Referrer-Policy", "no-referrer"),
        crate::header("X-Frame-Options", "SAMEORIGIN"),
        crate::header("X-Win11-Service", "rust-native"),
    ]
}
fn bytes(request: tiny_http::Request, status: u16, data: Vec<u8>, mime: &str) {
    let length = data.len();
    let _ = request.respond(tiny_http::Response::new(
        tiny_http::StatusCode(status),
        headers(mime),
        std::io::Cursor::new(data),
        Some(length),
        None,
    ));
}
fn text(request: tiny_http::Request, status: u16, message: &str) {
    bytes(
        request,
        status,
        format!("{message}\n").into_bytes(),
        "text/plain; charset=utf-8",
    )
}
fn json_response(request: tiny_http::Request, status: u16, data: Value) {
    bytes(
        request,
        status,
        data.to_string().into_bytes(),
        "application/json; charset=utf-8",
    )
}
fn decode_method(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = vec![];
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            out.push(u8::from_str_radix(&encoded[at + 1..at + 3], 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}
#[cfg(test)]
mod i18n_tests {
    use super::*;
    use crate::i18n::{self, Lang};

    // The 3b898e6 literals of runtime.rs and services/mod.rs: the Dutch render must stay byte-identical.
    const DUTCH: &[(&str, &str)] = &[
        ("core.serviceStopped", "De dienst is afgesloten."),
        ("core.badArgs", "Ongeldige invoer."),
        ("core.forbidden", "Geen toegang."),
        ("core.methodNotAllowed", "Gebruik POST."),
        ("core.badRequest", "Ongeldige aanvraag."),
        ("core.lengthRequired", "Ongeldige aanvraag."),
        ("core.tooLarge", "Te veel gegevens."),
        (
            "core.referenceUnreachable",
            "Referentiedienst niet bereikbaar",
        ),
        ("core.unknownMethod", "Deze functie bestaat niet."),
        ("core.starting", "De dienst wordt gestart."),
    ];

    #[test]
    fn core_texts_render_the_old_dutch_and_have_english() {
        for (key, nl) in DUTCH {
            assert_eq!(i18n::tr(Lang::Nl, key), *nl, "{key}");
            let en = i18n::tr(Lang::En, key);
            assert!(!en.is_empty() && en != *key, "{key} has no English");
        }
        let e = "connection refused";
        assert_eq!(
            i18n::trf(
                Lang::Nl,
                "core.referenceOffline",
                &json!({"error": e.to_string()})
            ),
            format!("Referentiedienst niet bereikbaar: {e}")
        );
        assert_eq!(
            i18n::trf(
                Lang::En,
                "core.referenceOffline",
                &json!({"error": e.to_string()})
            ),
            "Can’t reach the reference service: connection refused"
        );
        assert_eq!(i18n::tr(Lang::En, "core.forbidden"), "Access denied.");
        assert_eq!(
            i18n::tr(Lang::En, "core.unknownMethod"),
            "This function doesn’t exist."
        );
    }

    #[test]
    fn envelopes_carry_code_message_key_and_params() {
        // C2: code and message as before, plus key and params
        let nl = failure(files::ApiError::tr(
            "forbidden",
            Lang::Nl,
            "core.forbidden",
            i18n::NO_PARAMS,
        ));
        assert_eq!(
            nl.to_string(),
            r#"{"ok":false,"error":{"code":"forbidden","message":"Geen toegang.","key":"core.forbidden","params":{}}}"#
        );
        let en = failure(files::ApiError::tr(
            "offline",
            "en",
            "core.referenceOffline",
            &json!({"error": "timed out"}),
        ));
        assert_eq!(
            en["error"]["message"],
            "Can’t reach the reference service: timed out"
        );
        assert_eq!(en["error"]["params"], json!({"error": "timed out"}));
        // unknown(): data stays where it was
        let unknown = unknown(Lang::En, "x.y").json();
        assert_eq!(
            unknown,
            json!({"code":"unknown_method","message":"This function doesn’t exist.","key":"core.unknownMethod","params":{},"data":{"method":"x.y"}})
        );
        // an untranslated error keeps its old shape exactly
        assert_eq!(
            files::ApiError::new("gnome", "bericht").json().to_string(),
            r#"{"code":"gnome","message":"bericht"}"#
        );
        // envelope() of a method result
        let out = envelope(Err(unknown_error()), "x.y", false);
        assert_eq!(out["error"]["key"], "core.unknownMethod");
        assert_eq!(out["error"]["message"], "Deze functie bestaat niet.");
    }

    fn unknown_error() -> files::ApiError {
        unknown(Lang::Nl, "x.y")
    }

    #[test]
    fn plural_errors_report_n() {
        let e = files::ApiError::trn(
            "x",
            Lang::Nl,
            "core.noSuchPluralForTests",
            3u32,
            i18n::NO_PARAMS,
        );
        assert_eq!(e.params, Some(json!({"n": "3"})));
        assert_eq!(e.key.as_deref(), Some("core.noSuchPluralForTests"));
    }
}

#[cfg(test)]
mod methods_tests {
    use super::*;

    /// The prefixes each fragment may list (notes/claude/settings-parity.md §3, §5).
    const FRAGMENT_PREFIXES: &[(&str, &[&str])] = &[
        ("api-methods/foundation.json", &["gsettings.", "caps."]),
        ("api-methods/input.json", &["input."]),
        ("api-methods/devices.json", &["devices.", "printers."]),
        ("api-methods/network.json", &["net.", "sharing."]),
        (
            "api-methods/system.json",
            &["display.", "sound.", "battery.", "wellbeing.", "storage."],
        ),
        ("api-methods/accounts.json", &["users.", "goa.", "machine."]),
        (
            "api-methods/apps.json",
            &["privacy.", "background.", "notifications.", "apps."],
        ),
    ];

    #[test]
    fn the_old_list_loads_byte_for_byte_and_the_union_is_sorted_and_unique() {
        let old: Vec<Value> = serde_json::from_str(include_str!("api-methods.json")).unwrap();
        assert_eq!(
            load_methods(&[("api-methods.json", include_str!("api-methods.json"))]).unwrap(),
            old,
            "api-methods.json alone gives /api/methods as before"
        );
        let all = methods();
        let names: Vec<&str> = all.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted, "sorted by name, no name twice");
        // the drift of amendment 54
        assert!(is_known("system.power.settings.get") && is_known("system.power.settings.set"));
        assert!(
            all.iter()
                .any(|m| m["name"] == "system.power.settings.set" && m["dangerous"] == true)
        );
        assert!(is_known("gsettings.set") && is_known("caps.get"));
        assert!(!is_secret("gsettings.set"));
    }

    #[test]
    fn duplicates_and_malformed_entries_are_refused() {
        let a = r#"[{"name":"x.a","dangerous":false,"doc":""}]"#;
        let b = r#"[{"name":"x.a","dangerous":true,"doc":"again"}]"#;
        assert!(
            load_methods(&[("a", a), ("b", b)])
                .unwrap_err()
                .contains("x.a")
        );
        for bad in [
            r#"[{"name":"x.b","doc":""}]"#,
            r#"[{"name":"x.b","dangerous":"yes","doc":""}]"#,
            r#"[{"name":"x.b","dangerous":true,"doc":"","secret":1}]"#,
            r#"[{"name":"x.b","dangerous":true,"doc":"","extra":1}]"#,
            r#"[{"dangerous":true,"doc":""}]"#,
            r#"{"name":"x.b"}"#,
        ] {
            assert!(load_methods(&[("f", bad)]).is_err(), "{bad}");
        }
        let secret = r#"[{"name":"x.c","dangerous":true,"doc":"","secret":true}]"#;
        assert_eq!(load_methods(&[("f", secret)]).unwrap()[0]["secret"], true);
    }

    #[test]
    fn every_service_method_is_listed_with_its_dangerous_flag() {
        let all = methods();
        for (service, table) in crate::services::method_tables() {
            for (name, dangerous) in table {
                let entry = all.iter().find(|m| m["name"] == name);
                assert!(
                    entry.is_some(),
                    "{service}: {name} is in no api-methods file"
                );
                if let Some(dangerous) = dangerous {
                    assert_eq!(
                        entry.unwrap()["dangerous"],
                        dangerous,
                        "{service}: {name}'s dangerous flag"
                    );
                }
            }
        }
    }

    /// The other direction (review of B0, finding 15): a batch's new method is in its service's METHODS table too,
    /// so the flag test above sees its dangerous flag.
    #[test]
    fn every_fragment_method_is_in_its_services_table() {
        let tables = crate::services::method_tables();
        for (file, text) in METHOD_FILES.iter().skip(1) {
            for entry in load_methods(&[(file, text)]).unwrap() {
                let name = entry["name"].as_str().unwrap();
                let service = name.split('.').next().unwrap_or("");
                let (_, table) = tables
                    .iter()
                    .find(|(s, _)| *s == service)
                    .unwrap_or_else(|| panic!("{file}: {name}: no METHODS table for {service}"));
                assert!(
                    table.iter().any(|(n, _)| *n == name),
                    "{file}: {name} is not in {service}'s METHODS table"
                );
            }
        }
        for service in ["display", "storage", "notifications", "apps"] {
            assert!(
                tables.iter().any(|(s, t)| *s == service && !t.is_empty()),
                "{service}"
            );
        }
    }

    #[test]
    fn each_fragment_lists_only_its_batch_prefixes() {
        for (file, text) in METHOD_FILES.iter().skip(1) {
            let prefixes = FRAGMENT_PREFIXES
                .iter()
                .find(|(f, _)| f == file)
                .map(|(_, p)| *p)
                .unwrap_or_else(|| panic!("{file} has no prefix rule"));
            for entry in load_methods(&[(file, text)]).unwrap() {
                let name = entry["name"].as_str().unwrap();
                assert!(
                    prefixes.iter().any(|p| name.starts_with(p)),
                    "{file}: {name} is outside {prefixes:?}"
                );
            }
        }
    }
}
