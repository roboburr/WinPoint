//! Authenticated native HTTP/API runtime shared by the independent Rust windows.
use crate::{
    UiRequest, UiRequestKind, apps,
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
    ui_sender: std::sync::mpsc::Sender<UiRequest>,
    gui: bool,
    /// The last beat of the owner's GLib main loop (a 500 ms timer in every owner: lib.rs `run_inner` and
    /// `run_http_only`, portal.rs); `shell.info` reports its age as `mainLoopLagMs`. The `--http-only`
    /// backends run a main loop too since the native programs (lib.rs), so every runtime has one.
    beat: Mutex<Option<Instant>>,
}
impl Runtime {
    pub fn new(
        ui: PathBuf,
        kind: &'static str,
        ui_sender: std::sync::mpsc::Sender<UiRequest>,
        shutdown: Arc<dyn Fn(bool) + Send + Sync>,
        gui: bool,
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
            ui_sender,
            gui,
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
    pub fn beat(&self) {
        *self.beat.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    fn ui(&self, kind: UiRequestKind, args: &Value, timeout: Duration) -> files::ApiResult {
        let (ack, rx) = std::sync::mpsc::channel();
        self.ui_sender
            .send(UiRequest {
                kind,
                payload: args.clone(),
                ack,
            })
            .map_err(|_| {
                files::ApiError::tr(
                    "surface",
                    self.ctx.lang(),
                    "core.windowClosed",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        rx.recv_timeout(timeout)
            .map_err(|_| {
                files::ApiError::tr(
                    "surface",
                    self.ctx.lang(),
                    "core.windowNotResponding",
                    crate::i18n::NO_PARAMS,
                )
            })?
            .map_err(|m| files::ApiError::new("surface", &m))
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
        if method == "nativeBackdrop.configure" {
            return Some(if args.as_object().is_some_and(|args| !args.is_empty()) {
                Err(files::ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "core.noInput",
                    crate::i18n::NO_PARAMS,
                ))
            } else {
                Ok(gnome::configure_backdrop(
                    self.kind,
                    self.gui,
                    &self.ctx.settings(),
                ))
            });
        }
        match method{
            // D-G70: no WebKit shell surface has an input region any more; regions.js and the mock still ask
            "shell.setRegions"=>return Some(Ok(json!({}))),
            "shell.pickFile"=>return Some(if self.gui{self.ui(UiRequestKind::FilePicker,args,Duration::from_secs(1800))}else{Ok(json!({"path":null}))}),
            "explorer.tabPosition" if self.kind=="explorer"=>return Some(gnome::call("explorer.tabPosition",&json!({"pid":std::process::id(),"phase":if args["phase"]=="begin"{"begin"}else{"end"}})).map_err(|e|files::ApiError::new("gnome",&e))),
            _=>{}
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
        if let Some(relative) = path.strip_prefix("/ui/") {
            if !matches!(
                request.method(),
                tiny_http::Method::Get | tiny_http::Method::Head
            ) {
                return text(request, 405, "Method Not Allowed");
            }
            let target = url::Url::parse(&format!("file:///{relative}"))
                .ok()
                .and_then(|u| u.to_file_path().ok())
                .map(|p| self.ui.join(p.strip_prefix("/").unwrap_or(&p)));
            if let Some(target) = target.and_then(|p| p.canonicalize().ok()).filter(|p| {
                self.ui.canonicalize().is_ok_and(|root| p.starts_with(root)) && p.is_file()
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
            let hello = json!({"mode":if self.kind=="panel"{"layer"}else{"app"},"version":env!("CARGO_PKG_VERSION"),"dryRun":self.ctx.dry_run,"compositor":"gnome"});
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
fn methods() -> &'static Vec<Value> {
    static METHODS: OnceLock<Vec<Value>> = OnceLock::new();
    METHODS.get_or_init(|| {
        serde_json::from_str(include_str!("api-methods.json")).expect("embedded API methods")
    })
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
        ("core.windowClosed", "Venster is gesloten"),
        ("core.windowNotResponding", "Venster reageert niet"),
        ("core.badArgs", "Ongeldige invoer."),
        ("core.noInput", "Deze actie accepteert geen invoer."),
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
