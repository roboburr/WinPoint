//! The Windows Update service `update.*` (notes/claude/windows-update-gnome.md §4).
//!
//! * **Modes** (design §4.1). A failed test seam → `unsupported` (`test_seam`): no bus, no thread, no tool, for the
//!   whole process life. A dry-run host → `dry`: files only, every write answers its dry-run envelope. Every kind
//!   but `settings` → `reader`: files only, writes are `not_here`. A `settings` host takes `update/owner.lock`
//!   (non-blocking flock, kept for its life): the winner is the **owner** (full mode, the `update-pk` thread);
//!   a loser is a **mirror** that reads files and forwards its writes as `update.request`, retries the lock every
//!   30 s and on `update.owner {state:'gone'}`.
//! * The writes of design §3.7's list first run the release-upgrader detection (`update.guard.upgrade`); cancel,
//!   details, a declined confirm, setAutoOpen, ubuntuHold and attach stay possible (review F17, §4.5 1a).
//! * Every argument goes through a per-method allowlist (the pattern of `software.rs`). Ids come only from the
//!   snapshot, flags are fixed, and there is no install/remove/repo method.
//! * Texts through `crate::i18n` (`host/i18n/update.tsv`); the state files store codes, keys and params only.
#[path = "update/admin.rs"]
mod admin;
#[path = "update/compat.rs"]
mod compat;
#[path = "update/elf.rs"]
mod elf;
#[path = "update/guard.rs"]
pub(crate) mod guard;
#[path = "update/history.rs"]
mod history;
#[path = "update/model.rs"]
pub(crate) mod model;
#[path = "update/net.rs"]
pub(crate) mod net;
#[path = "update/options.rs"]
mod options;
#[path = "update/paths.rs"]
pub(crate) mod paths;
#[path = "update/phasing.rs"]
pub(crate) mod phasing;
#[path = "update/pk.rs"]
mod pk;
#[path = "update/release.rs"]
mod release;
#[path = "update/restart.rs"]
pub(crate) mod restart;
#[path = "update/store.rs"]
pub(crate) mod store;
#[path = "update/upgrade.rs"]
mod upgrade;
#[cfg(test)]
#[path = "update/bus_tests.rs"]
mod bus_tests;
#[cfg(test)]
#[path = "update/tests.rs"]
mod tests;

use super::ServiceContext;
use crate::{
    files::{ApiError, ApiResult},
    i18n::Lang,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex, Weak, mpsc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

/// What a service did, for the tests (dry-run and seam failures: all zero).
#[derive(Debug, Default)]
pub(crate) struct Stats {
    /// every bus connection: the PackageKit/NetworkManager target and the session bus of `notify()` (review F9)
    pub bus_opens: AtomicUsize,
    pub threads: AtomicUsize,
    pub forwarded: AtomicUsize,
    /// the keep-alive of design §4.1 (C2) is held: while our transaction runs, and after a job until the
    /// re-check answered (`pk::Engine::hold_app`; the bus tests read it, review F2/F7)
    pub held: AtomicBool,
    /// the throttle's clock reading of every fan-out `update.progress` (`Delivery::Both`; the flood bus test
    /// checks "at most 1 per second" on it, review m4)
    #[cfg(test)]
    pub fanout: Mutex<Vec<std::time::Instant>>,
    /// the session-bus unique name of each `Notify` this service sent (the bus tests match it with the fake's
    /// `sender`, review m6)
    #[cfg(test)]
    pub notified: Mutex<Vec<String>>,
}

/// What the pk thread, the guard and the later steps' modules share.
pub(crate) struct Shared {
    pub ctx: Arc<ServiceContext>,
    pub setup: paths::Setup,
    pub store: store::Store,
    pub stats: Stats,
    pub rev: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Owner,
    Mirror,
    Reader,
    Dry,
    Unsupported,
}

pub(crate) struct Update {
    ctx: Arc<ServiceContext>,
    shared: Option<Arc<Shared>>,
    dry: bool,
    mode: Mutex<Mode>,
    owner_lock: Mutex<Option<store::FileLock>>,
    pk: Mutex<Option<Arc<pk::Handle>>>,
    stopping: AtomicBool,
    weak: Weak<Self>,
    // GW7: the 5 s release hand-over watch is armed (one timer per service)
    upgrade_watch: AtomicBool,
}

/// The methods, their argument keys, and whether they write (writes are forwarded, guarded and dry-run'd).
const METHODS: &[(&str, &[&str], bool)] = &[
    ("update.status", &["packages"], false),
    ("update.history", &["before", "limit"], false),
    ("update.options", &[], false),
    ("update.check", &[], true),
    ("update.details", &["ids"], true),
    ("update.install", &["rows"], true),
    ("update.confirm", &["jobId", "accept"], true),
    ("update.cancel", &["jobId"], true),
    ("update.repair", &[], true),
    ("update.setAutoOpen", &["on"], true),
    ("update.release.check", &[], true),
    ("update.release.compat", &["refresh"], true),
    ("update.release.setPrompt", &["prompt"], true),
    ("update.release.ubuntuHold", &["on"], true),
    // GW7: `anyway` is the page's "Toch upgraden…" after its warning (hold with allowAnyway only, design §6.4)
    ("update.release.start", &["anyway"], true),
    ("update.release.attach", &[], true),
    // the native Settings: whether it shows a window (its keep-alive, native/apps/settings/src/keepalive.rs); local to
    // this host, never forwarded
    ("update.setWatching", &["on"], false),
];
const MAX_DETAIL_IDS: usize = 20;
const MIRROR_RETRY_SECONDS: u32 = 30;
/// A bus test's shorter mirror retry (0 = [`MIRROR_RETRY_SECONDS`]): the takeover after an owner crash (review m6).
#[cfg(test)]
pub(crate) static MIRROR_RETRY_TEST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
fn mirror_retry_seconds() -> u32 {
    #[cfg(test)]
    {
        let seconds = MIRROR_RETRY_TEST.load(Ordering::SeqCst);
        if seconds > 0 {
            return seconds;
        }
    }
    MIRROR_RETRY_SECONDS
}
/// How long `shutdown()` waits for the pk thread to drop its engine (no join; design §9.2: shutdown < 100 ms).
const SHUTDOWN_WAIT: Duration = Duration::from_millis(80);

fn spec(method: &str) -> Option<(&'static [&'static str], bool)> {
    METHODS.iter().find(|(name, _, _)| *name == method).map(|(_, keys, write)| (*keys, *write))
}
/// The writes refused while Ubuntu's release upgrader runs: design §3.7's list for `update.guard.upgrade`
/// (check, install, confirm, repair, release.check/compat/setPrompt/start). Cancelling or declining our own
/// job, reading details, the two settings toggles and attaching to the upgrader stay possible, because §4.5 1a
/// lets an own job keep running then (review F17).
pub(crate) fn upgrade_guarded(method: &str, args: &Value) -> bool {
    match method {
        "update.check" | "update.install" | "update.repair" | "update.release.check" | "update.release.compat"
        | "update.release.setPrompt" | "update.release.start" => true,
        "update.confirm" => args["accept"] != false,
        _ => false,
    }
}
fn product(lang: Lang) -> String {
    crate::i18n::tr(lang, "update.product")
}
fn bad_args(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "update.err.badArgs", crate::i18n::NO_PARAMS)
}
fn seam_error(lang: Lang) -> ApiError {
    ApiError::tr("test_seam", lang, "update.err.testSeam", crate::i18n::NO_PARAMS)
}
/// The stubs of GW4–GW7 answer this until their step fills them (the message is the method: no user text).
pub(crate) fn not_yet(method: &str) -> ApiError {
    ApiError::new("unsupported", method)
}

/// The argument allowlist of each method (keys and types); ids and rows are checked again against the snapshot.
pub(crate) fn check_args(method: &str, args: &Value) -> bool {
    let Some((keys, _)) = spec(method) else { return false };
    let Some(object) = args.as_object() else { return false };
    if object.keys().any(|k| !keys.contains(&k.as_str())) {
        return false;
    }
    let token = |v: &Value| v.as_str().is_some_and(store::is_token);
    match method {
        "update.status" => object.get("packages").is_none_or(Value::is_boolean),
        "update.history" => {
            object.get("before").is_none_or(|v| v.as_u64().is_some())
                && object.get("limit").is_none_or(|v| v.as_u64().is_some_and(|n| (1..=200).contains(&n)))
        }
        "update.details" => args["ids"].as_array().is_some_and(|ids| {
            (1..=MAX_DETAIL_IDS).contains(&ids.len())
                && ids.iter().all(|id| id.as_str().is_some_and(|s| s.len() <= 512 && s.split(';').count() == 4))
        }),
        "update.install" => object.get("rows").is_none_or(|rows| {
            rows.as_array().is_some_and(|rows| {
                !rows.is_empty()
                    && rows.len() <= model::ROW_ORDER.len()
                    && rows.iter().all(|r| r.as_str().is_some_and(|r| model::ROW_ORDER.contains(&r)))
                    && rows.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>().len() == rows.len()
            })
        }),
        "update.confirm" => token(&args["jobId"]) && args["accept"].is_boolean(),
        "update.cancel" => token(&args["jobId"]),
        "update.setAutoOpen" | "update.release.ubuntuHold" | "update.setWatching" => args["on"].is_boolean(),
        "update.release.compat" => object.get("refresh").is_none_or(Value::is_boolean),
        "update.release.setPrompt" => matches!(args["prompt"].as_str(), Some("lts" | "normal" | "never")),
        // GW7
        "update.release.start" => object.get("anyway").is_none_or(Value::is_boolean),
        _ => true,
    }
}

impl Update {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Self::with_env(ctx, &paths::Env::process()))
    }
    /// The service for an explicit environment (unit tests inject the `WIN11_UPDATE_*` variables and the euid).
    pub(crate) fn with_env(ctx: Arc<ServiceContext>, env: &paths::Env) -> Arc<Self> {
        let dry = ctx.dry_run || crate::backend::dry_run();
        let shared = paths::resolve(env).ok().and_then(|setup| {
            let store = store::Store::open(ctx.cache_dir.join("update")).ok()?;
            Some(Arc::new(Shared { ctx: ctx.clone(), setup, store, stats: Stats::default(), rev: AtomicU64::new(0) }))
        });
        let mode = match &shared {
            None => Mode::Unsupported,
            Some(_) if dry => Mode::Dry,
            Some(_) if ctx.kind != "settings" => Mode::Reader,
            Some(_) => Mode::Mirror,
        };
        let service = Arc::new_cyclic(|weak| Self {
            ctx: ctx.clone(),
            shared,
            dry,
            mode: Mutex::new(mode),
            owner_lock: Mutex::new(None),
            pk: Mutex::new(None),
            stopping: AtomicBool::new(false),
            weak: weak.clone(),
            // GW7
            upgrade_watch: AtomicBool::new(false),
        });
        if mode == Mode::Mirror {
            // this host shows the update page: update.progress and update.reply reach its WebViews; every other
            // host only parses them (design §4.4, review F6). The backend of the native Settings
            // (`win11-app --app settings --http-only`) is such a host: its event stream carries them to
            // win11-gtk-settings, whose keep-alive follows them (native/apps/settings/src/keepalive.rs, design C2
            // below at `hold_application`). retry_later's timeout runs on that backend's main loop (lib.rs
            // run_http_only).
            ctx.events.show_update_events();
            service.observe();
            if !service.take_over() {
                service.retry_later();
            }
        }
        service
    }
    pub(crate) fn mode(&self) -> Mode {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner())
    }
    #[cfg(test)]
    pub(crate) fn stats(&self) -> Option<(usize, usize, usize, usize)> {
        let s = self.shared.as_ref()?;
        Some((
            s.stats.bus_opens.load(Ordering::SeqCst),
            s.stats.threads.load(Ordering::SeqCst),
            s.setup.tools.runs(),
            s.stats.forwarded.load(Ordering::SeqCst),
        ))
    }

    // ------------------------------------------------------------------------------------ owner election

    /// Settings hosts listen for forwarded writes (the owner runs them) and for the owner leaving (a mirror
    /// takes over). Only `settings` hosts act on these events (design §4.4).
    fn observe(&self) {
        let weak = self.weak.clone();
        self.ctx.events.on(
            "update.request",
            Arc::new(move |data| {
                if let Some(service) = weak.upgrade() {
                    service.on_request(data);
                }
            }),
        );
        let weak = self.weak.clone();
        self.ctx.events.on(
            "update.owner",
            Arc::new(move |data| {
                if data["state"] == "gone"
                    && let Some(service) = weak.upgrade()
                    && service.mode() == Mode::Mirror
                {
                    service.take_over();
                }
            }),
        );
    }
    /// Try `owner.lock`; on a win become the owner and start the pk thread (which reattaches, §4.7).
    fn take_over(&self) -> bool {
        let Some(shared) = self.shared.clone() else { return false };
        if self.stopping.load(Ordering::SeqCst) || self.mode() != Mode::Mirror {
            return false;
        }
        let Ok(store::Election::Won(lock)) = store::elect(&shared.store) else { return false };
        *self.owner_lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(lock);
        match pk::Handle::start(shared) {
            Ok(handle) => {
                *self.pk.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(handle));
                *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Owner;
                // GW7: a new owner picks up a release hand-over that is still unresolved
                self.watch_upgrade();
                true
            }
            Err(_) => {
                self.owner_lock.lock().unwrap_or_else(|e| e.into_inner()).take();
                false
            }
        }
    }
    /// A mirror retries the lock every 30 s: a timeout on the main loop, not a thread (design §4.1).
    fn retry_later(&self) {
        let weak = self.weak.clone();
        glib::timeout_add_seconds(mirror_retry_seconds(), move || match weak.upgrade() {
            Some(service) if service.mode() == Mode::Mirror && !service.stopping.load(Ordering::SeqCst) => {
                if service.take_over() { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            }
            _ => glib::ControlFlow::Break,
        });
    }
    /// A forwarded write (mirror → owner): validated with the same allowlists, run, answered as `update.reply`.
    fn on_request(&self, data: &Value) {
        if self.mode() != Mode::Owner || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Some(nonce) = data["nonce"].as_str().filter(|n| store::is_token(n)).map(str::to_owned) else { return };
        let method = data["method"].as_str().unwrap_or("").to_owned();
        let args = data["args"].clone();
        let weak = self.weak.clone();
        // off the event receiver thread: a write may wait up to 5 s for the pk thread
        std::thread::spawn(move || {
            let Some(service) = weak.upgrade() else { return };
            let lang = service.ctx.lang();
            let result = if spec(&method).is_some_and(|(_, write)| write) && check_args(&method, &args) {
                service.write(Mode::Owner, &method, &args, lang)
            } else {
                Err(bad_args(lang))
            };
            let reply = match result {
                Ok(_) => json!({"nonce":nonce,"ok":true}),
                Err(error) => json!({"nonce":nonce,"ok":false,"error":error.json()}),
            };
            service.ctx.emit("update.reply", reply);
        });
    }

    // ------------------------------------------------------------------------------------ calls

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let (_, write) = spec(method)?;
        let lang = self.ctx.lang();
        if !check_args(method, args) {
            return Some(Err(bad_args(lang)));
        }
        if method == "update.setWatching" {
            // the job's notification (pk.rs notify) asks ServiceContext::watching
            self.ctx.set_windows_shown(args["on"] == true);
            return Some(Ok(json!({"on": args["on"]})));
        }
        let mode = self.mode();
        if mode == Mode::Unsupported {
            return Some(if method == "update.status" { Ok(self.unsupported_status(lang)) } else { Err(seam_error(lang)) });
        }
        let shared = self.shared.as_ref()?;
        if !write {
            return Some(match method {
                "update.status" => Ok(self.status(args["packages"] == true)),
                "update.history" => history::history(shared, args, lang),
                _ => options::options(shared, lang),
            });
        }
        Some(self.write(mode, method, args, lang))
    }
    fn write(&self, mode: Mode, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let Some(shared) = self.shared.as_ref() else { return Err(seam_error(lang)) };
        if mode == Mode::Dry || self.dry {
            return Ok(self.dry_result(method));
        }
        // GW7: the hand-over watch runs once on every update write (design §6.4 step 5)
        if mode == Mode::Owner {
            self.watch_upgrade();
        }
        if upgrade_guarded(method, args) && guard::upgrader_running(&shared.setup.paths) {
            return Err(ApiError::tr("update_busy", lang, "update.guard.upgrade", crate::i18n::NO_PARAMS));
        }
        // GW7: a check after the card showed startFailed/ended returns the card to the offer and its checks
        if mode == Mode::Owner && method == "update.check" {
            upgrade::clear_shown(shared);
        }
        match mode {
            Mode::Owner => self.run(shared, method, args, lang),
            Mode::Mirror => self.forward(shared, method, args, lang),
            _ => Err(ApiError::tr("not_here", lang, "update.err.notHere", crate::i18n::NO_PARAMS)),
        }
    }
    /// The dry-run envelopes of design §4.3: no D-Bus, no program, no gsettings write.
    fn dry_result(&self, method: &str) -> Value {
        match method {
            "update.check" => {
                let mut view = self.status(false);
                view["dryRun"] = json!(true);
                view
            }
            "update.details" => json!({"details":[],"dryRun":true}),
            "update.install" => json!({"jobId":null,"dryRun":true}),
            "update.release.check" => json!({"release":self.status(false)["release"],"dryRun":true}),
            "update.release.compat" => json!({"state":"unknown","dryRun":true}),
            _ => json!({"dryRun":true}),
        }
    }
    /// A mirror never runs a write itself: `update.request` to the owner, which answers with `update.reply`
    /// (an immediate refusal) and `update.changed` (the result).
    fn forward(&self, shared: &Shared, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let nonce = store::token().map_err(|_| bad_args(lang))?;
        shared.stats.forwarded.fetch_add(1, Ordering::SeqCst);
        self.ctx.emit("update.request", json!({"nonce":nonce,"method":method,"args":args}));
        Ok(json!({"forwarded":true,"nonce":nonce}))
    }
    fn handle(&self) -> Option<Arc<pk::Handle>> {
        self.pk.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn run(&self, shared: &Shared, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let failed = |key: &str| ApiError::tr("failed", lang, key, &json!({"product": product(lang)}));
        let ask = |f: Box<dyn FnOnce(&mut pk::Engine) -> ApiResult + Send>, key: &str| {
            self.handle().and_then(|h| h.ask(f)).unwrap_or_else(|| Err(failed(key)))
        };
        let job_id = args["jobId"].as_str().unwrap_or("").to_owned();
        match method {
            "update.check" => ask(Box::new(move |e| e.check(lang)), "update.err.checkFailed"),
            "update.install" => {
                let rows = args["rows"].as_array().map(|r| r.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
                ask(Box::new(move |e| e.install(rows, lang)), "update.err.failed")
            }
            "update.confirm" => {
                let accept = args["accept"] == true;
                ask(Box::new(move |e| e.confirm(&job_id, accept, lang)), "update.err.failed")
            }
            "update.cancel" => ask(Box::new(move |e| e.cancel(&job_id, lang)), "update.err.cannotCancel"),
            "update.repair" => ask(Box::new(move |e| e.repair(lang)), "update.err.failed"),
            "update.details" => self.details(args, lang),
            "update.setAutoOpen" => options::set_auto_open(shared, args, lang),
            "update.release.check" => release::check(shared, lang),
            "update.release.compat" => compat::compat(shared, args, lang),
            "update.release.setPrompt" => release::set_prompt(shared, args, lang),
            "update.release.ubuntuHold" => release::ubuntu_hold(shared, args, lang),
            // GW7: the page's `anyway`, and the 5 s watch once the terminal is on its way
            "update.release.start" => {
                let result = upgrade::start(shared, args, lang);
                if result.is_ok() {
                    self.watch_upgrade();
                }
                result
            }
            "update.release.attach" => upgrade::attach(shared, lang),
            _ => Err(bad_args(lang)),
        }
    }
    /// `update.details`: the pk thread answers from its cache or after GetUpdateDetail; after 5 s the cached
    /// part with `pending:true` (the page asks again; changelogs are packagekitd's downloads).
    fn details(&self, args: &Value, lang: Lang) -> ApiResult {
        let ids: Vec<String> = args["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let Some(handle) = self.handle() else {
            return Err(ApiError::tr("details", lang, "update.err.details", crate::i18n::NO_PARAMS));
        };
        let (reply, answer) = mpsc::sync_channel(1);
        let asked = ids.clone();
        handle.ask(move |e| e.details(asked, lang, reply));
        match answer.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => {
                let cached = handle.ask(move |e| e.cached_details(&ids)).unwrap_or_default();
                Ok(json!({"details":cached,"pending":true}))
            }
        }
    }

    // ------------------------------------------------------------------------------------ status

    /// `update.status`: the files (snapshot, job, reboot-required, the upgrader scan) plus, in the owner, the
    /// pk thread's live job progress.
    fn status(&self, packages: bool) -> Value {
        let Some(shared) = self.shared.as_ref() else { return self.unsupported_status(self.ctx.lang()) };
        let mode = self.mode();
        // GW7: the hand-over watch runs once on every page open (design §6.4 step 5)
        if mode == Mode::Owner {
            self.watch_upgrade();
        }
        let live = (mode == Mode::Owner).then(|| self.handle().and_then(|h| h.ask(|e| e.live()))).flatten().flatten();
        let snapshot = shared.store.read(store::SNAPSHOT);
        let job = shared.store.read(store::JOB);
        let mut view = compose(shared, snapshot.as_ref(), job.as_ref(), live.as_ref(), packages);
        // GW7: the card has shown startFailed/ended once this answer leaves (upgrade.json may go at the next check)
        if mode == Mode::Owner {
            upgrade::shown(shared, &view);
        }
        view["owner"]["mirror"] = json!(mode == Mode::Mirror);
        view["mode"] = json!(match mode {
            Mode::Owner => "owner",
            Mode::Mirror => "mirror",
            Mode::Reader => "reader",
            Mode::Dry => "dry",
            Mode::Unsupported => "unsupported",
        });
        if self.dry {
            view["dryRun"] = json!(true);
            if snapshot.is_none() {
                view["backend"] = json!("packagekit");
            }
        }
        view
    }
    /// A failed seam: `unsupported` with `error.code = test_seam`, built from nothing (no file is read).
    fn unsupported_status(&self, lang: Lang) -> Value {
        let error = seam_error(lang);
        json!({"backend":null,"phase":"unsupported","rev":0,"checkedAt":null,"refreshing":false,"owner":null,
               "mode":"unsupported","rows":[],"totals":model::totals(&[], None),"job":null,
               "restart":{"reboot":false,"rebootPkgs":[],"gnome":false,"shell":false,"shellParts":[],"since":null},
               "error":{"code":error.code,"message":error.message,"key":error.key,"params":{},"detail":""},
               "warnings":[],"network":net::Net::default().json(),"release":{"state":"none"},"dryRun":self.dry})
    }

    // GW7: the release hand-over watch (design §6.4 step 5), owner only: once now, then every 5 s on the main loop
    // (a timeout, not a thread) while upgrade.json is unresolved. Unit tests call `upgrade::watch` themselves.
    fn watch_upgrade(&self) {
        let Some(shared) = self.shared.clone() else { return };
        if self.mode() != Mode::Owner || self.stopping.load(Ordering::SeqCst) || !upgrade::watch(&shared) {
            return;
        }
        if cfg!(test) || self.upgrade_watch.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = self.weak.clone();
        glib::timeout_add_seconds(upgrade::WATCH_SECONDS, move || {
            let Some(service) = weak.upgrade() else { return glib::ControlFlow::Break };
            let go = service.mode() == Mode::Owner
                && !service.stopping.load(Ordering::SeqCst)
                && service.shared.as_ref().is_some_and(|shared| upgrade::watch(shared));
            if go {
                glib::ControlFlow::Continue
            } else {
                service.upgrade_watch.store(false, Ordering::SeqCst);
                glib::ControlFlow::Break
            }
        });
    }

    pub(crate) fn shutdown(&self) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let was_owner = self.mode() == Mode::Owner;
        if let Some(handle) = self.pk.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // no join: a bounded wait (≤ 80 ms) for the pk thread to drop its engine, so the job lock and a test
            // bus connection are gone before a mirror takes over (design §4.1, §9.2 shutdown < 100 ms)
            handle.stop(SHUTDOWN_WAIT);
        }
        self.owner_lock.lock().unwrap_or_else(|e| e.into_inner()).take();
        if was_owner {
            *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Reader;
            self.ctx.emit("update.owner", json!({"pid":std::process::id(),"state":"gone"}));
        }
    }
    /// An owner that crashed, as every other process sees it (bus tests, review m6): the pk thread and its test bus
    /// connection end and `owner.lock` is free, but no `update.owner {gone}` goes out, so a mirror only takes over
    /// through its retry (design §4.1).
    #[cfg(test)]
    pub(crate) fn crash(&self) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(handle) = self.pk.lock().unwrap_or_else(|e| e.into_inner()).take() {
            handle.stop(SHUTDOWN_WAIT);
        }
        self.owner_lock.lock().unwrap_or_else(|e| e.into_inner()).take();
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Reader;
    }
}

/// Render a stored error/warning record `{code, key, params, detail}` in `lang` (design §8: files store codes).
fn render(record: &Value, lang: Lang) -> Value {
    let Some(key) = record["key"].as_str().filter(|k| k.starts_with("update.")) else {
        return Value::Null;
    };
    let mut params = record["params"].as_object().cloned().unwrap_or_default();
    params.entry("product").or_insert_with(|| json!(product(lang)));
    let params = Value::Object(params);
    json!({"code":record["code"],"message":crate::i18n::trf(lang, key, &params),"key":key,"params":record["params"],
           "detail":record["detail"]})
}

/// os-release `VERSION` (re-rooted in test mode).
pub(crate) fn os_release(paths: &paths::Paths) -> String {
    std::fs::read_to_string(&paths.os_release)
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("VERSION="))
                .map(|v| v.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_default()
}
/// Is a locale available (libc `newlocale`, no `locale -a` program)? Cached per name.
pub(crate) fn locale_available(name: &str) -> bool {
    static CACHE: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, found)) = cache.iter().find(|(n, _)| n == name) {
        return *found;
    }
    let found = std::ffi::CString::new(name).is_ok_and(|c| {
        let locale = unsafe { libc::newlocale(libc::LC_ALL_MASK, c.as_ptr(), std::ptr::null_mut()) };
        if locale.is_null() {
            false
        } else {
            unsafe { libc::freelocale(locale) };
            true
        }
    });
    cache.push((name.to_owned(), found));
    found
}
/// A GNOME notification (design §3.8): `org.freedesktop.Notifications.Notify` on the session bus, app name
/// `{product}`, hint `desktop-entry`. Owner only, never in dry-run.
pub(crate) fn notify(shared: &Shared, title: &str, body: &str) {
    if shared.ctx.dry_run || crate::backend::dry_run() {
        return;
    }
    shared.stats.bus_opens.fetch_add(1, Ordering::SeqCst);
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    let lang = shared.ctx.lang();
    let hints = glib::VariantDict::new(None);
    hints.insert("desktop-entry", "io.github.win11shell.Settings");
    let args = (product(lang), 0u32, "system-software-update", title, body, Vec::<String>::new(), hints.end(), -1i32);
    use gio::prelude::*;
    bus.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&glib::Variant::tuple_from_iter([
            args.0.to_variant(),
            args.1.to_variant(),
            args.2.to_variant(),
            args.3.to_variant(),
            args.4.to_variant(),
            args.5.to_variant(),
            args.6,
            args.7.to_variant(),
        ])),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        |_| {},
    );
    // the keep-alive may end soon after this: the message must have left the process (review F2)
    let _ = bus.flush_sync(gio::Cancellable::NONE);
    #[cfg(test)]
    shared.stats.notified.lock().unwrap_or_else(|e| e.into_inner()).push(bus.unique_name().map(|n| n.to_string()).unwrap_or_default());
}
thread_local! {
    static HOLD: std::cell::RefCell<Option<gio::ApplicationHoldGuard>> = const { std::cell::RefCell::new(None) };
}
/// The keep-alive of design §4.1 (C2): `Application::hold()` on the GTK main thread while our job runs, so
/// closing the Settings window drops the window but keeps the process until `Finished`. No application
/// (tests, `--http-only`): nothing happens.
///
/// The native Settings: the job's owner is its backend (`win11-app --app settings --http-only`), which has no
/// GApplication and dies with the Settings process (PR_SET_PDEATHSIG, native/kit/src/backend.rs). So
/// win11-gtk-settings holds its own GApplication (native/apps/settings/src/keepalive.rs): it follows `update.changed`,
/// `update.progress` and `update.owner` and reads `update.status`; while its backend is the owner and a job of ours
/// runs (not paused for a confirm), and after it until the re-check answered (`refreshing`, at most 60 s as
/// HOLD_AFTER_JOB), closing the window keeps the process and its backend. Then it exits.
pub(crate) fn hold_application(on: bool) {
    glib::MainContext::default().invoke(move || {
        use gio::prelude::*;
        HOLD.with(|hold| {
            let mut hold = hold.borrow_mut();
            if on {
                if hold.is_none() {
                    *hold = gio::Application::default().map(|app| app.hold());
                }
            } else {
                hold.take();
            }
        });
    });
}

/// The snapshot of `update.status` from the files and an optional live job (design §4.3).
pub(crate) fn compose(shared: &Shared, snapshot: Option<&Value>, job: Option<&Value>, live: Option<&Value>, packages: bool) -> Value {
    let lang = shared.ctx.lang();
    let paths = &shared.setup.paths;
    let empty = json!({});
    let snap = snapshot.unwrap_or(&empty);
    let boot = store::boot_id(paths);
    let pkgs: Vec<model::Pkg> = snap["packages"].as_array().map(|a| a.iter().filter_map(model::Pkg::from_json).collect()).unwrap_or_default();
    let os = model::os_version(&os_release(paths));
    let rows = model::group(&pkgs, &os);
    // our job: the live view (owner), else an unfinished job file of this boot whose owner is alive
    let file_job = job.filter(|j| {
        j.get("finishedAt").is_none_or(Value::is_null)
            && boot.is_some()
            && j["bootId"].as_str() == boot.as_deref()
            && store::owner_alive(&j["owner"])
    });
    let job_view = live.cloned().or_else(|| {
        file_job.map(|j| {
            json!({"jobId":j["jobId"],"role":j["role"],"status":j["lastStatus"],"pct":j["pct"],"speed":0,"remaining":0,
                   "downloadLeft":null,"allowCancel":j["allowCancel"],"committing":j["committing"],"startedAt":j["startedAt"],
                   "reattached":j["reattached"],"foreign":false,"confirm":j["confirm"],"rows":j["rowStates"],
                   "simulating":j["simulating"] == true,"downloadSize":j["simulate"]["downloadSize"]})
        })
    });
    // totals.downloadSize: the snapshot's (the simulate's DownloadSizeRemaining), else what the running job knows
    // (the first DownloadSizeRemaining of its download), so the hero can show "{done} van {total}" (GW3 D1)
    // (0 is "unknown", never a total: review m1)
    let download_size = snap["downloadSize"]
        .as_u64()
        .filter(|size| *size > 0)
        .or_else(|| job_view.as_ref().and_then(|j| j["downloadSize"].as_u64()).filter(|size| *size > 0));
    let totals = model::totals(&pkgs, download_size);
    let stage = job_view.as_ref().map(|j| {
        model::stage(j["status"].as_u64().unwrap_or(0) as u32, j["committing"] == true, j["simulating"] == true || j["confirm"].is_object())
    });
    // restart: the reboot flag, a RequireRestart of this boot, the maps scan
    let reboot = restart::reboot(paths);
    let require = require_restart(snap, boot.as_deref());
    let mut reboot_pkgs = reboot.pkgs.clone();
    for name in require.and_then(|r| r["pkgs"].as_array()).into_iter().flatten().filter_map(Value::as_str) {
        if !reboot_pkgs.iter().any(|p| p == name) {
            reboot_pkgs.push(name.to_owned());
        }
    }
    // the rows: our running job, else the last job's record until the next check (review F3)
    let last_job = snap.get("lastJob").filter(|j| j.is_object());
    let mut rows_json = model::row_views(&rows, job_view.as_ref(), last_job, &reboot_pkgs);
    for row in rows_json.iter_mut() {
        if row["error"].is_object() {
            row["error"] = render(&row["error"], lang);
        }
    }
    let scan = &snap["scan"];
    let gnome = scan["gnome"] == true;
    let restart_json = json!({"reboot":reboot.required || require.is_some(),"rebootPkgs":reboot_pkgs,"gnome":gnome,
                              "shell":scan["shell"] == true,"shellParts":scan["shellParts"].as_array().cloned().unwrap_or_default(),
                              "since":reboot.since});
    let owner = shared.store.read(store::OWNER);
    // "checking" only while the owner that wrote it lives: a Settings process that ended during a re-check never
    // leaves readers on "checking" (review F2)
    let checking = snap["checking"] == true
        && owner.as_ref().is_some_and(|o| store::owner_alive(o) && o["pid"] == snap["owner"]["pid"]);
    let net = net::Net::from_json(&snap["network"]);
    let release_running = guard::upgrader_running(paths);
    let error = render(&snap["error"], lang);
    let warnings: Vec<Value> = snap["warnings"].as_array().into_iter().flatten().map(|w| render(w, lang)).filter(|w| !w.is_null()).collect();
    let phase = model::derive_phase(&model::PhaseInput {
        unsupported: snapshot.is_some() && snap["backend"].is_null(),
        release_running,
        job: stage,
        foreign: snap["foreign"] == true,
        repair_required: snap["repairRequired"] == true,
        restart_required: reboot.required || require.is_some() || gnome,
        error: !error.is_null(),
        offline: !net.online,
        checking,
        available: totals["count"].as_u64().unwrap_or(0) as usize,
    });
    let phase = if phase == model::Phase::Unsupported && error.is_null() {
        (model::Phase::Unsupported, json!({"code":"unsupported","key":"update.err.noPackageKit","params":{},"detail":""}))
    } else {
        (phase, Value::Null)
    };
    let error = if !phase.1.is_null() { render(&phase.1, lang) } else { error };
    let waiting_lock = job_view.as_ref().is_some_and(|j| j["status"] == model::status::WAITING_FOR_LOCK);
    let unattended = waiting_lock && guard::unattended_running(paths);
    let foreign_own = phase.0 == model::Phase::Foreign && guard::software_job_active(&shared.ctx.cache_dir.join("software/jobs"));
    // The page's job fields (GW3 D1, D-G41): `simulating`, `lockHolder` ('unattended' while unattended-upgrades
    // holds the dpkg lock we wait for, §4.7), `foreignOwn` (the foreign transaction is one of our own Software
    // jobs, §3.2), `downloadSize`. A foreign transaction without a job of ours gets a job object of its own, so the
    // hero can tell `foreign` from `foreignOwn` (the mock's H09/H26 shape).
    let job_json = match job_view {
        Some(mut j) => {
            j["lockHolder"] = if unattended { json!("unattended") } else { Value::Null };
            j["foreignOwn"] = json!(false);
            j
        }
        None if phase.0 == model::Phase::Foreign => {
            json!({"jobId":null,"role":null,"status":null,"pct":null,"speed":0,"remaining":0,"downloadLeft":null,
                   "allowCancel":false,"committing":false,"startedAt":null,"reattached":false,"foreign":true,
                   "foreignOwn":foreign_own,"lockHolder":null,"confirm":null,"rows":{},"simulating":false,
                   "downloadSize":null})
        }
        None => Value::Null,
    };
    let mut release = release::status(shared);
    // GW7: GW6's verdict, before the running state below replaces it
    let verdict = release["state"].clone();
    if release_running {
        release["state"] = json!("running");
    }
    // GW7: the hand-over overlay (design §3.5, §6.4): running/waiting/startFailed/ended > updatesFirst > rebootFirst
    // (only while the verdict allows a start) > the verdict; startRc, reasons, heldNames, attachable
    upgrade::overlay(shared, &mut release, verdict, snap, release_running);
    let mut view = json!({
        "backend": snap.get("backend").cloned().unwrap_or(Value::Null),
        "phase": phase.0.as_str(),
        "rev": snap["rev"].as_u64().unwrap_or(0),
        "checkedAt": snap["checkedAt"],
        "refreshing": checking,
        "owner": {"pid": owner.as_ref().and_then(|o| o["pid"].as_u64()), "mirror": false},
        "rows": rows_json,
        "totals": totals,
        "job": job_json,
        "restart": restart_json,
        "error": error,
        "warnings": warnings,
        "network": net.json(),
        "release": release,
        // kept from GW2a (additive); the page reads job.foreignOwn and job.lockHolder
        "foreignOwn": foreign_own,
        "waitLockAuto": unattended,
        "phasing": snap.get("phasing").cloned().unwrap_or(Value::Null),
        "dryRun": false,
    });
    if packages {
        view["packages"] = json!(pkgs.iter().map(model::Pkg::json).collect::<Vec<_>>());
    }
    view
}

/// The snapshot's `RequireRestart` record when it applies (this boot; a reboot or a new session, design §4.8).
fn require_restart<'a>(snapshot: &'a Value, boot: Option<&str>) -> Option<&'a Value> {
    snapshot.get("requireRestart").filter(|r| restart::require_restart_applies(r, boot))
}
/// `restart.reboot` of `update.status`, `rebootRequired` of the owner's `system.updates.changed` and of
/// `system.updates`: one rule (review m5). `/run/reboot-required`, or a `RequireRestart` record of this boot.
fn reboot_flag(snapshot: &Value, paths: &paths::Paths) -> bool {
    restart::reboot(paths).required || require_restart(snapshot, store::boot_id(paths).as_deref()).is_some()
}

/// `system.updates` from the snapshot once one exists (design §4.10; `system/info.rs` `Updates::get`/`check`, GW2c).
/// None (info.rs keeps its `apt list` path): no snapshot, or one that never finished a check (`checkedAt` is not
/// a number, as in the snapshot an owner writes when Settings › Update first opens), or one without PackageKit
/// (`backend` not `packagekit`: §4.5 "backend:'apt' then keeps the count"; review F4). A failed test seam
/// answers a null count, so a misconfigured test never reaches `apt list` either (fail closed).
pub(crate) fn system_updates(ctx: &ServiceContext) -> Option<Value> {
    let Ok(setup) = paths::process_setup() else {
        return Some(json!({"available":null,"checkedAt":null,"tool":null,"security":null,"checking":false,"rebootRequired":false}));
    };
    let store = store::Store::open(ctx.cache_dir.join("update")).ok()?;
    system_updates_from(store.read(store::SNAPSHOT)?, &setup.paths)
}
/// The pure part of [`system_updates`].
pub(crate) fn system_updates_from(snapshot: Value, paths: &paths::Paths) -> Option<Value> {
    let checked_at = snapshot["checkedAt"].as_u64()?;
    if snapshot["backend"] != "packagekit" {
        return None;
    }
    let pkgs: Vec<model::Pkg> = snapshot["packages"].as_array().map(|a| a.iter().filter_map(model::Pkg::from_json).collect()).unwrap_or_default();
    let totals = model::totals(&pkgs, None);
    Some(json!({"available":totals["count"],"checkedAt":checked_at,"tool":"packagekit","security":totals["security"],
                "checking":false,"rebootRequired":reboot_flag(&snapshot, paths)}))
}
/// `/run/reboot-required` through the path table (re-rooted in test mode); false after a failed seam.
pub(crate) fn reboot_required() -> bool {
    paths::process_setup().is_ok_and(|setup| setup.paths.reboot_required.exists())
}
