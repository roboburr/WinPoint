//! Everything owns one separate native index worker. Other hosts never start it.
#[path = "index/cache.rs"]
mod cache;
#[path = "index/engine.rs"]
mod engine;
#[path = "index/query.rs"]
mod query;
use super::ServiceContext;
use crate::{
    files::{ApiError, ApiResult},
    i18n::{self, Lang},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
struct Worker {
    child: Child,
    stdin: ChildStdin,
}
pub(crate) struct Index {
    ctx: Arc<ServiceContext>,
    weak: Weak<Self>,
    worker: Mutex<Option<Worker>>,
    pending: Mutex<HashMap<u64, mpsc::SyncSender<Result<Value, Value>>>>,
    next: AtomicU64,
    stopped: AtomicBool,
    last_status: Mutex<Value>,
    crashes: Mutex<VecDeque<Instant>>,
    gave_up: AtomicBool,
    last_config: Mutex<Value>,
}
impl Index {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let ix = Arc::new_cyclic(|weak| Self {
            ctx,
            weak: weak.clone(),
            worker: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            last_status: Mutex::new(Value::Null),
            crashes: Mutex::new(VecDeque::new()),
            gave_up: AtomicBool::new(false),
            last_config: Mutex::new(Value::Null),
        });
        let weak = Arc::downgrade(&ix);
        thread::spawn(move || {
            let mut lang = None;
            loop {
                thread::sleep(Duration::from_millis(500));
                let Some(ix) = weak.upgrade() else { break };
                if ix.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let cfg = ix.config();
                let changed = {
                    let mut prev = ix.last_config.lock().unwrap_or_else(|e| e.into_inner());
                    let changed = *prev != cfg;
                    *prev = cfg.clone();
                    changed
                };
                if changed {
                    let _ = ix.send(&json!({"method":"configure","args":cfg}));
                }
                // a language switch: an open Alles window gets the status again in the new language
                let now = ix.ctx.lang();
                if lang.replace(now).is_some_and(|before| before != now) {
                    ix.resend_status(now);
                }
            }
        });
        if ix.ctx.kind == "alles"
            && std::env::var("WIN11SHELL_INDEX_AUTOSTART").as_deref() == Ok("1")
        {
            ix.ensure()?;
        }
        Ok(ix)
    }
    fn config(&self) -> Value {
        let settings = self.ctx.settings();
        let s = &settings["alles"];
        let override_roots = std::env::var("WIN11SHELL_INDEX_ROOTS")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| json!(s.split(':').filter(|s| !s.is_empty()).collect::<Vec<_>>()));
        let roots =
            override_roots.unwrap_or_else(|| s.get("roots").cloned().unwrap_or(json!("auto")));
        let roots = if let Some(list) = roots.as_array() {
            let values = list
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>();
            if values.is_empty() {
                json!("auto")
            } else {
                json!(values)
            }
        } else {
            json!("auto")
        };
        let excluded = s["excluded"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({"roots":roots,"excluded":excluded,"liveUpdate":s.get("liveUpdate").map(query::truth).unwrap_or(true)})
    }
    fn worker_path() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("WIN11_INDEX_WORKER") {
            return Ok(path.into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(exe.with_file_name("win11-index-worker"))
    }
    fn ensure(&self) -> Result<(), String> {
        if self.ctx.kind != "alles"
            || self.stopped.load(Ordering::Relaxed)
            || self.gave_up.load(Ordering::Relaxed)
        {
            return Err("index_unavailable".into());
        }
        let mut slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = slot.as_mut() {
            if w.child.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Ok(());
            }
        }
        if let Some(mut previous) = slot.take() {
            let _ = previous.child.wait();
            for (_, tx) in self
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
            {
                let _=tx.send(Err(json!({"code":"index_restarting","message":crate::i18n::tr(self.ctx.lang(), "index.restarting"),"key":"index.restarting","params":{}})));
            }
        }
        let path = Self::worker_path()?;
        let mut command = Command::new(path);
        command
            .arg("--cache")
            .arg(&self.ctx.cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command.env_remove("WIN11SHELL_TOKEN");
        let parent = std::process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent as i32 {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        let mut child = spawn_worker(command).map_err(|e| e.to_string())?;
        let pid = child.id();
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| crate::i18n::tr(self.ctx.lang(), "index.noStdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| crate::i18n::tr(self.ctx.lang(), "index.noStdout"))?;
        let config = self.config();
        writeln!(stdin, "{}", json!({"method":"configure","args":config}))
            .map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        *slot = Some(Worker { child, stdin });
        drop(slot);
        let weak = self.weak.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.len() > 64 * 1024 * 1024 {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(ix) = weak.upgrade() else { break };
                if let Some(event) = msg["event"].as_str() {
                    let data = msg.get("data").cloned().unwrap_or(json!({}));
                    if event == "status" {
                        *ix.last_status.lock().unwrap_or_else(|e| e.into_inner()) = data.clone();
                        ix.ctx.emit("index.status", localized(&data, ix.ctx.lang()));
                    } else if event == "changed" {
                        ix.ctx.emit("index.changed", data);
                    }
                    continue;
                }
                if let Some(id) = msg["id"].as_u64() {
                    if let Some(tx) = ix
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id)
                    {
                        let _ = tx.send(if msg["ok"] == true {
                            Ok(msg["result"].clone())
                        } else {
                            Err(msg["error"].clone())
                        });
                    }
                }
            }
            if let Some(ix) = weak.upgrade() {
                ix.worker_exited(pid);
            }
        });
        Ok(())
    }
    fn worker_exited(&self, pid: u32) {
        let mut slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_none_or(|w| w.child.id() != pid) {
            return;
        }
        if let Some(mut w) = slot.take() {
            let _ = w.child.wait();
        }
        drop(slot);
        for (_, tx) in self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
        {
            let _=tx.send(Err(json!({"code":"index_restarting","message":crate::i18n::tr(self.ctx.lang(), "index.restarting"),"key":"index.restarting","params":{}})));
        }
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let count = {
            let mut crashes = self.crashes.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            crashes.push_back(now);
            while crashes
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(300))
            {
                crashes.pop_front();
            }
            crashes.len()
        };
        if count >= 5 {
            self.gave_up.store(true, Ordering::Relaxed);
            self.ctx
                .emit("index.status", gave_up_status(self.ctx.lang()));
            return;
        }
        let weak = self.weak.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(
                (1u64 << count.saturating_sub(1)).min(30),
            ));
            if let Some(ix) = weak.upgrade() {
                if !ix.stopped.load(Ordering::Relaxed) {
                    let _ = ix.ensure();
                }
            }
        });
    }
    /// Sends the last status again in `lang` (after a language switch), also when the worker gave up or is gone,
    /// so the message and the root errors Alles shows follow the switch (round2-gate §2).
    fn resend_status(&self, lang: Lang) {
        if self.gave_up.load(Ordering::Relaxed) {
            self.ctx.emit("index.status", gave_up_status(lang));
            return;
        }
        let last = self
            .last_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if last.is_object() {
            self.ctx.emit("index.status", localized(&last, lang));
        }
    }
    fn send(&self, value: &Value) -> Result<(), String> {
        let mut slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        let Some(w) = slot.as_mut() else {
            return Err(crate::i18n::tr(self.ctx.lang(), "index.notStarted").into());
        };
        writeln!(w.stdin, "{value}")
            .and_then(|_| w.stdin.flush())
            .map_err(|e| e.to_string())
    }
    fn request(&self, method: &str, args: &Value) -> ApiResult {
        if self.ensure().is_err() {
            return Err(if self.ctx.kind != "alles" {
                ApiError::tr(
                    "index_unavailable",
                    self.ctx.lang(),
                    "index.openAlles",
                    crate::i18n::NO_PARAMS,
                )
            } else {
                ApiError::tr(
                    "index_unavailable",
                    self.ctx.lang(),
                    "index.unavailable",
                    crate::i18n::NO_PARAMS,
                )
            });
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        if self
            .send(&json!({"id":id,"method":method,"args":args}))
            .is_err()
        {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(ApiError::tr(
                "index_unavailable",
                self.ctx.lang(),
                "index.restarting",
                crate::i18n::NO_PARAMS,
            ));
        }
        let result = rx.recv_timeout(Duration::from_secs(if method == "search" { 20 } else { 8 }));
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(worker_error(&error, self.ctx.lang())),
            Err(_) => {
                if method == "search" {
                    if let Some(w) = self
                        .worker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_mut()
                    {
                        let _ = w.child.kill();
                    }
                }
                Err(ApiError::tr(
                    "timeout",
                    self.ctx.lang(),
                    "index.timeout",
                    crate::i18n::NO_PARAMS,
                ))
            }
        }
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        Some(match method {
            "index.status" => match self.request("status", &json!({})) {
                Ok(st) => {
                    let shown = localized(&st, self.ctx.lang());
                    *self.last_status.lock().unwrap_or_else(|e| e.into_inner()) = st;
                    Ok(shown)
                }
                Err(e) => {
                    let mut st = localized(
                        &self.last_status.lock().unwrap_or_else(|e| e.into_inner()),
                        self.ctx.lang(),
                    );
                    if !st.is_object() {
                        st = json!({"state":"error","files":0,"folders":0,"roots":[],"builtAt":null,"progress":null,"watching":0});
                    }
                    st["message"] = json!(e.message);
                    Ok(st)
                }
            },
            "index.search" => {
                let mut a = if args.is_object() {
                    args.clone()
                } else {
                    json!({})
                };
                let count = if a.get("limit").is_some_and(Value::is_null)
                    || a.get("offset").is_some_and(Value::is_null)
                {
                    Err(ApiError::tr(
                        "bad_args",
                        self.ctx.lang(),
                        "index.badArgs",
                        crate::i18n::NO_PARAMS,
                    ))
                } else {
                    query::integer(&a["limit"], 200).and_then(|limit| {
                        query::integer(&a["offset"], 0).map(|offset| (limit, offset))
                    })
                };
                match count {
                    Err(_) => Err(ApiError::tr(
                        "bad_args",
                        self.ctx.lang(),
                        "index.badArgs",
                        crate::i18n::NO_PARAMS,
                    )),
                    Ok((limit, offset)) => {
                        a["limit"] = json!(limit.clamp(0, 1000));
                        a["offset"] = json!(offset.max(0));
                        if a.get("query").is_some_and(|v| !v.is_string()) {
                            Err(ApiError::tr(
                                "bad_args",
                                self.ctx.lang(),
                                "index.badQuery",
                                crate::i18n::NO_PARAMS,
                            ))
                        } else {
                            if !a.as_object().unwrap().contains_key("showHidden") {
                                a["showHidden"] = json!(query::truth(
                                    &self.ctx.settings()["alles"]["showHidden"]
                                ));
                            }
                            self.request("search", &a)
                        }
                    }
                }
            }
            "index.rebuild" => self
                .request("rebuild", &json!({}))
                .map(|_| json!({"ok":true})),
            "index.setRoots" => self.set_roots(args),
            "index.locations" => Ok(self.locations()),
            _ => return None,
        })
    }
    fn set_roots(&self, args: &Value) -> ApiResult {
        let roots = args.get("roots").cloned().unwrap_or(json!("auto"));
        let roots = if roots == "auto" {
            roots
        } else {
            let Some(roots) = roots.as_array() else {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "index.noRoots",
                    crate::i18n::NO_PARAMS,
                ));
            };
            if !roots.iter().all(Value::is_string) {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "index.noRoots",
                    crate::i18n::NO_PARAMS,
                ));
            }
            let mut clean = Vec::new();
            for r in roots {
                let s = r.as_str().unwrap();
                let path = cache::normalize(s.trim());
                let real = fs::canonicalize(&path).unwrap_or_else(|_| path.into());
                if !real.is_dir() {
                    return Err(ApiError::tr(
                        "not_found",
                        self.ctx.lang(),
                        "index.noSuchFolder",
                        &serde_json::json!({"path": s.to_string()}),
                    ));
                }
                let path = real.to_string_lossy().into_owned();
                if !clean.contains(&path) {
                    clean.push(path)
                }
            }
            if clean.is_empty() {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "index.noLocation",
                    crate::i18n::NO_PARAMS,
                ));
            }
            json!(clean)
        };
        let mut patch = json!({"alles":{"roots":roots}});
        if let Some(ex) = args.get("excluded") {
            let Some(ex) = ex.as_array() else {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "index.badExcluded",
                    crate::i18n::NO_PARAMS,
                ));
            };
            if !ex.iter().all(Value::is_string) {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "index.badExcluded",
                    crate::i18n::NO_PARAMS,
                ));
            }
            patch["alles"]["excluded"] = json!(
                ex.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(cache::normalize)
                    .collect::<Vec<_>>()
            );
        }
        self.ctx.call("settings.patch", &json!({"patch":patch}))?;
        let _ = self.send(&json!({"method":"configure","args":self.config()}));
        if std::env::var("WIN11SHELL_INDEX_ROOTS").is_ok_and(|v| !v.is_empty()) {
            Ok(
                json!({"ok":true,"overridden":true,"message":crate::i18n::tr(self.ctx.lang(), "index.rootsOverridden")}),
            )
        } else {
            Ok(json!({"ok":true}))
        }
    }
    fn locations(&self) -> Value {
        let lang = self.ctx.lang();
        let mounts = cache::mounts();
        let auto = cache::auto_roots(&mounts);
        let cfg = self.config();
        let roots = cfg["roots"].as_array().map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(|s| {
                    fs::canonicalize(cache::normalize(s))
                        .unwrap_or_else(|_| cache::normalize(s).into())
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
        });
        let active = roots.as_ref().unwrap_or(&auto);
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut labels = HashMap::new();
        if let Ok(entries) = fs::read_dir("/dev/disk/by-label") {
            for e in entries.flatten() {
                if let Ok(real) = fs::canonicalize(e.path()) {
                    labels.insert(
                        real.to_string_lossy().into_owned(),
                        decode_label(&e.file_name().to_string_lossy()),
                    );
                }
            }
        }
        for m in mounts {
            if seen.contains(&m.path)
                || !cache::REAL_FS.contains(&m.fs.as_str())
                || m.path != "/" && !auto.contains(&m.path)
            {
                continue;
            }
            seen.insert(m.path.clone());
            let (total, free) = statvfs(&m.path);
            let source = fs::canonicalize(&m.source)
                .unwrap_or_else(|_| m.source.clone().into())
                .to_string_lossy()
                .into_owned();
            let basename = Path::new(m.path.trim_end_matches('/'))
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| m.path.clone());
            let label = if m.path == "/" {
                crate::i18n::tr(lang, "index.systemDrive").into()
            } else if let Some(label) = labels.get(&source) {
                label.clone()
            } else if basename.len() >= 8
                && basename.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                && total > 0
            {
                crate::i18n::trf(
                    lang,
                    "index.sizedDrive",
                    &serde_json::json!({"size": (size_label(lang, total)).to_string()}),
                )
            } else {
                basename
            };
            let disk = cache::disk(&m.source);
            let removable = !disk.is_empty()
                && (fs::read_to_string(format!("/sys/block/{disk}/removable"))
                    .is_ok_and(|s| s.trim() == "1")
                    || fs::canonicalize(format!("/sys/block/{disk}"))
                        .is_ok_and(|p| p.to_string_lossy().contains("/usb")));
            out.push(json!({"path":m.path,"label":label,"fsType":m.fs,"total":total,"free":free,"auto":auto.contains(&m.path),"indexed":active.contains(&m.path),"kind":"drive","removable":removable}));
        }
        if let Some(roots) = roots {
            let home = fs::canonicalize(glib::home_dir())
                .unwrap_or_else(|_| glib::home_dir())
                .to_string_lossy()
                .into_owned();
            for p in roots {
                if seen.insert(p.clone()) {
                    let label = Path::new(&p)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.clone());
                    out.push(json!({"path":p,"label":label,"fsType":"","total":0,"free":0,"auto":false,"indexed":true,"kind":"folder","removable":false,"isHome":p==home}));
                }
            }
        }
        json!(out)
    }
    pub(crate) fn shutdown(&self) {
        if self.stopped.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = self.send(&json!({"id":0,"method":"shutdown"}));
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let done = {
                let mut slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
                slot.as_mut()
                    .is_none_or(|w| w.child.try_wait().ok().flatten().is_some())
            };
            if done || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        if let Some(mut w) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if w.child.try_wait().ok().flatten().is_none() {
                let _ = w.child.kill();
            }
            let _ = w.child.wait();
        }
    }
}
type SpawnRequest = (Command, mpsc::SyncSender<std::io::Result<Child>>);

/// Start the worker from one long-lived thread. PR_SET_PDEATHSIG fires when the *thread* that forked the child
/// ends, and requests run on short-lived HTTP threads (runtime.rs serve), so a worker spawned by a request was
/// killed right after its answer (and Alles gave up after five "crashes").
fn spawn_worker(mut command: Command) -> std::io::Result<Child> {
    static SPAWNER: std::sync::OnceLock<Option<Mutex<mpsc::Sender<SpawnRequest>>>> =
        std::sync::OnceLock::new();
    let spawner = SPAWNER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<SpawnRequest>();
        thread::Builder::new()
            .name("index-spawner".into())
            .spawn(move || {
                for (mut command, reply) in rx {
                    let _ = reply.send(command.spawn());
                }
            })
            .ok()
            .map(|_| Mutex::new(tx))
    });
    let Some(tx) = spawner else {
        return command.spawn();
    };
    let (reply, answer) = mpsc::sync_channel(1);
    if let Err(mpsc::SendError((mut command, _))) = tx
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send((command, reply))
    {
        return command.spawn();
    }
    answer
        .recv()
        .unwrap_or_else(|_| Err(std::io::Error::other("index-spawner")))
}

/// The worker's status in `lang`. The worker sends a root error as text plus its catalogue key (`errorKey`,
/// `errorParams`); the host renders it again in its own language (C1) whenever it relays or returns a status, so a
/// status kept for later is never frozen in the language it was made in (round2-gate §2).
fn localized(status: &Value, lang: Lang) -> Value {
    let mut st = status.clone();
    if let Some(roots) = st.get_mut("roots").and_then(Value::as_array_mut) {
        for root in roots {
            if let Some(key) = root["errorKey"].as_str().map(str::to_owned) {
                root["error"] = json!(i18n::trf(lang, &key, &root["errorParams"]));
            }
        }
    }
    st
}
/// An error the worker sent (`{"code", "message", "key"?, "params"?}`). A catalogue error (C2) is rendered again
/// from its key + params in this host's language (C1): the worker renders in its own process language only.
fn worker_error(error: &Value, lang: Lang) -> ApiError {
    let code = error["code"].as_str().unwrap_or("internal");
    match error["key"].as_str() {
        Some(key) => ApiError::tr(code, lang, key, &error["params"]),
        None => ApiError::new(
            code,
            error["message"]
                .as_str()
                .unwrap_or(&crate::i18n::tr(lang, "index.searchFailed")),
        ),
    }
}
/// The status Alles shows after the worker crashed five times in five minutes.
fn gave_up_status(lang: Lang) -> Value {
    json!({"state":"error","files":0,"folders":0,"roots":[],"builtAt":null,"progress":null,"watching":0,"message":crate::i18n::tr(lang, "index.gaveUp")})
}
fn statvfs(path: &str) -> (u64, u64) {
    let Ok(c) = std::ffi::CString::new(path) else {
        return (0, 0);
    };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
        (
            s.f_blocks.saturating_mul(s.f_frsize),
            s.f_bavail.saturating_mul(s.f_frsize),
        )
    } else {
        (0, 0)
    }
}
fn decode_label(label: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let b = label.as_bytes();
    while i < b.len() {
        if b[i..].starts_with(b"\\x") && i + 3 < b.len() {
            if let Ok(v) = u8::from_str_radix(&label[i + 2..i + 4], 16) {
                out.push(v as char);
                i += 4;
                continue;
            }
        }
        let c = label[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8()
    }
    out
}
fn size_label(lang: Lang, n: u64) -> String {
    if n >= 1 << 40 {
        let mut s = format!("{:.1}", n as f64 / (1u64 << 40) as f64);
        if s.ends_with(".0") {
            s.truncate(s.len() - 2)
        }
        // 3b898e6 always wrote a decimal comma ("1,5 TB"): Dutch keeps it, English gets a point
        let s = i18n::decimal(lang, &s);
        format!("{s} TB")
    } else {
        format!("{:.0} GB", n as f64 / (1u64 << 30) as f64)
    }
}
pub(crate) fn worker_main() {
    engine::main()
}
// R2-B: the index stem in English through its call sites; a status is stored as key + params and rendered when
// it is relayed or read, and it is sent again on a language switch (round2-gate §2).
#[cfg(test)]
mod i18n_tests {
    use super::*;
    use std::sync::mpsc;

    fn context(root: &Path, lang: &str, kind: &str) -> Arc<ServiceContext> {
        let config = root.join(format!("config-{lang}"));
        fs::create_dir_all(&config).unwrap();
        write_language(&config, lang);
        ServiceContext::new(
            crate::events::Events::new(&root.join(format!("events-{lang}"))).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            kind.into(),
            true,
        )
        .unwrap()
    }
    fn write_language(config: &Path, lang: &str) {
        let temp = config.join("settings.json.new");
        fs::write(&temp, json!({"language":lang}).to_string()).unwrap();
        fs::rename(&temp, config.join("settings.json")).unwrap();
    }
    fn error(result: Option<ApiResult>) -> ApiError {
        result.expect("an index method").expect_err("an error")
    }
    fn status_with_root_error() -> Value {
        // what the worker sends: its own rendering plus the key (engine.rs status())
        json!({"state":"error","files":0,"folders":0,"roots":[
            {"path":"/gone","files":0,"folders":0,"state":"error","error":"Deze locatie is niet gevonden.",
             "errorKey":"index.rootNotFound","errorParams":{}},
            {"path":"/io","files":0,"folders":0,"state":"error","error":"Permission denied (os error 13)"}]})
    }

    #[test]
    fn index_errors_render_in_english_and_dutch_through_the_service() {
        let root =
            std::env::temp_dir().join(format!("win11-index-i18n-{}", crate::events::nonce()));
        for (lang, open, roots, missing) in [
            (
                "en",
                "Open Alles to search your files.",
                "Choose one or more folders to search.",
                "The folder “/nonexistent-r2b-index” doesn’t exist.",
            ),
            (
                "nl",
                "Open Alles om bestanden te doorzoeken.",
                "Kies een of meer mappen om te doorzoeken.",
                "De map “/nonexistent-r2b-index” bestaat niet.",
            ),
        ] {
            // a host that is not Alles never starts the worker
            let ctx = context(&root, lang, "desktop");
            let ix = Index::new(ctx.clone()).unwrap();
            let e = error(ix.call("index.search", &json!({"query":"x"})));
            assert_eq!(
                (e.code.as_str(), e.message.as_str()),
                ("index_unavailable", open)
            );
            assert_eq!(e.key.as_deref(), Some("index.openAlles"));
            let st = ix.call("index.status", &json!({})).unwrap().unwrap();
            assert_eq!(st["message"], open);
            assert_eq!(
                error(ix.call("index.setRoots", &json!({"roots":5}))).message,
                roots
            );
            let e = error(ix.call(
                "index.setRoots",
                &json!({"roots":["/nonexistent-r2b-index"]}),
            ));
            assert_eq!(e.message, missing);
            assert_eq!(e.params, Some(json!({"path":"/nonexistent-r2b-index"})));
            ix.shutdown();
            ctx.events.shutdown();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn worker_errors_and_root_errors_are_rendered_in_the_hosts_language() {
        // the worker side: a parser error carries its key and params (query.rs), sent as e.json()
        let sent = query::Compiled::new("size:abc", &json!({}))
            .err()
            .unwrap()
            .json();
        assert_eq!(sent["key"], "index.query.sizeValue");
        let e = worker_error(&sent, Lang::En);
        assert_eq!(
            e.message,
            "I don’t understand the size “abc”. Try size:>10mb, for example."
        );
        let e = worker_error(&sent, Lang::Nl);
        assert_eq!(
            e.message,
            "Ik begrijp de grootte “abc” niet. Probeer bijvoorbeeld size:>10mb."
        );
        // a worker error without a key keeps its message; none at all: the host's own text
        assert_eq!(
            worker_error(&json!({"code":"x","message":"raw"}), Lang::En).message,
            "raw"
        );
        assert_eq!(
            worker_error(&json!({"code":"x"}), Lang::En).message,
            "Something went wrong while searching."
        );
        // root errors of the status
        let en = localized(&status_with_root_error(), Lang::En);
        assert_eq!(en["roots"][0]["error"], "This location can’t be found.");
        assert_eq!(en["roots"][1]["error"], "Permission denied (os error 13)");
        let nl = localized(&en, Lang::Nl);
        assert_eq!(nl["roots"][0]["error"], "Deze locatie is niet gevonden.");
        assert_eq!(localized(&Value::Null, Lang::En), Value::Null);
        // a build that fails on the index file: the cache's Fault keeps its key up to the root error
        let dir =
            std::env::temp_dir().join(format!("win11-index-fault-{}", crate::events::nonce()));
        fs::create_dir_all(dir.join("index")).unwrap();
        fs::write(dir.join("index/meta.json"), r#"{"version":1}"#).unwrap();
        let failed = cache::Data::load(&dir).err().unwrap();
        let root_error = cache::root_error(&failed);
        assert_eq!(root_error.key.as_deref(), Some("index.cache.otherVersion"));
        assert_eq!(
            crate::i18n::tr(Lang::En, root_error.key.as_deref().unwrap()),
            "Different index version"
        );
        let _ = fs::remove_dir_all(dir);
        // the location labels (size with the language's decimal separator)
        assert_eq!(size_label(Lang::Nl, 3 << 39), "1,5 TB");
        assert_eq!(size_label(Lang::En, 3 << 39), "1.5 TB");
        assert_eq!(size_label(Lang::Nl, 1 << 41), "2 TB");
        assert_eq!(size_label(Lang::En, 512 << 30), "512 GB");
    }

    #[test]
    fn a_language_switch_sends_the_status_again_in_the_new_language() {
        let root =
            std::env::temp_dir().join(format!("win11-index-switch-{}", crate::events::nonce()));
        let ctx = context(&root, "nl", "desktop");
        let ix = Index::new(ctx.clone()).unwrap();
        *ix.last_status.lock().unwrap() = status_with_root_error();
        let (tx, rx) = mpsc::channel::<Value>();
        let tx = Mutex::new(tx);
        ctx.events.on(
            "index.status",
            Arc::new(move |data| {
                let _ = tx.lock().unwrap().send(data.clone());
            }),
        );
        // let the watcher see Dutch first (it polls every 500 ms), then switch
        thread::sleep(Duration::from_millis(1200));
        write_language(&ctx.config_dir, "en");
        let sent = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("index.status after the switch");
        assert_eq!(sent["roots"][0]["error"], "This location can’t be found.");
        // the stored status is unchanged (key + params), so a later read renders it again
        assert_eq!(
            ix.last_status.lock().unwrap()["roots"][0]["error"],
            "Deze locatie is niet gevonden."
        );
        // after five crashes the gave-up status is the one that is sent again
        ix.gave_up.store(true, Ordering::Relaxed);
        write_language(&ctx.config_dir, "nl");
        let sent = loop {
            let v = rx
                .recv_timeout(Duration::from_secs(3))
                .expect("index.status after the second switch");
            if v["message"].is_string() {
                break v;
            }
        };
        assert_eq!(
            sent["message"],
            "Alles kon het overzicht van je bestanden niet bijhouden."
        );
        assert_eq!(
            gave_up_status(Lang::En)["message"],
            "Alles couldn’t keep track of your files."
        );
        ix.shutdown();
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
