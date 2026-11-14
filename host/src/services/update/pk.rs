//! The event-driven PackageKit client and the engine of the `update-pk` thread (design §4.1, §4.6–§4.8).
//!
//! * One thread `update-pk` owns a `glib::MainContext` + `MainLoop` (thread default). Every D-Bus reply,
//!   signal, FileMonitor event and timer of the service runs there; the engine lives in a thread-local, so no
//!   callback captures anything that is not `Send`. HTTP threads only `ask()` (invoke + a reply channel,
//!   at most 5 s) and never touch D-Bus.
//! * No poll loop and no time cap: a transaction ends on `Finished`, `Destroy` or the daemon vanishing. A 30 s
//!   watchdog calls `GetAll` only when a transaction sent nothing for 120 s.
//! * Reads that must not start packagekitd (`GetTransactionList`, `Properties.GetAll`, NetworkManager) use
//!   `NO_AUTO_START`; name presence uses `NameHasOwner` + `ListActivatableNames`.
use super::{
    Shared, guard, model,
    model::{Delivery, Lru, Pkg, Progress},
    net::{self, Net},
    paths::BusTarget,
    phasing, restart,
    store::{self, JOB, JOB_LOCK, SNAPSHOT},
};
use crate::{files::ApiError, files::ApiResult, i18n::Lang};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub(crate) const SERVICE: &str = "org.freedesktop.PackageKit";
const PATH: &str = "/org/freedesktop/PackageKit";
const IFACE: &str = "org.freedesktop.PackageKit";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const DBUS: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
/// Replies an HTTP thread waits for (design §4.1: ≤ 5 s; long operations report through events).
const ASK_TIMEOUT: Duration = Duration::from_secs(5);
const WATCHDOG: Duration = Duration::from_secs(30);
const SILENT: Duration = Duration::from_secs(120);
/// `job.json` is rewritten at least this often while a job runs (the power guard's dead-owner rule).
const JOB_REWRITE: Duration = Duration::from_secs(60);
const MAX_PACKAGES: usize = 10_000;
const MAX_DETAIL: usize = 4000;
const DETAILS_PER_CALL: usize = 20;
/// The `update.details` cache holds at most this many ids (each up to about 70 KB; review F15).
const DETAILS_CACHE: usize = 64;
/// After a job, the keep-alive waits at most this long for the re-check's answer (review F2).
const HOLD_AFTER_JOB: Duration = Duration::from_secs(60);
/// While a transaction waits for the dpkg lock, who holds it (unattended-upgrades?) is read again at most this
/// often (a `/proc` scan; review M1).
const HOLDER_RECHECK: Duration = Duration::from_secs(2);
/// `UpdatesChanged` → the list is read again this long after the last such signal (review m2).
const REREAD_DELAY: Duration = Duration::from_secs(2);

/// The real system bus. Under `cfg(test)` it panics unless `WIN11_UPDATE_PK_BUS` is set, and then it is that
/// private bus (after the seam check): a unit test can never reach the real daemon (design §4.11).
pub(crate) fn system_bus() -> Result<gio::DBusConnection, glib::Error> {
    #[cfg(test)]
    {
        let env = super::paths::Env::process();
        let Some(BusTarget::Address(address)) = super::paths::resolve(&env).ok().map(|s| s.bus) else {
            panic!("update::pk::system_bus() in a test without a valid WIN11_UPDATE_PK_BUS");
        };
        return address_bus(&address);
    }
    #[cfg(not(test))]
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
}
fn address_bus(address: &str) -> Result<gio::DBusConnection, glib::Error> {
    gio::DBusConnection::for_address_sync(
        address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
}
/// The connection for a checked target; `Disabled` never connects.
fn connect(target: &BusTarget) -> Option<gio::DBusConnection> {
    match target {
        BusTarget::System => system_bus().ok(),
        BusTarget::Address(address) => address_bus(address).ok(),
        BusTarget::Disabled => None,
    }
}

/// Is PackageKit there? (`NameHasOwner`, then `ListActivatableNames`; neither starts it.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Running,
    Activatable,
    Absent,
}
fn presence(conn: &gio::DBusConnection) -> Presence {
    let call = |method: &str, args: Option<glib::Variant>| {
        conn.call_sync(
            Some(DBUS),
            DBUS_PATH,
            DBUS,
            method,
            args.as_ref(),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()
    };
    if call("NameHasOwner", Some((SERVICE,).to_variant())).and_then(|v| v.child_value(0).get::<bool>()) == Some(true) {
        return Presence::Running;
    }
    let activatable = call("ListActivatableNames", None)
        .and_then(|v| v.child_value(0).get::<Vec<String>>())
        .is_some_and(|names| names.iter().any(|n| n == SERVICE));
    if activatable { Presence::Activatable } else { Presence::Absent }
}

// ------------------------------------------------------------------------------------------- thread + ask

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static ON_PK_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Run `f` on the engine (pk thread only). None when there is no engine or it is busy (never re-entered).
fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut engine| engine.as_mut().map(f)))
}

/// The HTTP side of the pk thread.
pub(crate) struct Handle {
    context: glib::MainContext,
    main_loop: glib::MainLoop,
    /// one message when the pk thread has dropped its engine (subscriptions, job lock, a test bus connection)
    done: std::sync::Mutex<Option<mpsc::Receiver<()>>>,
}
impl Handle {
    /// Start `update-pk`. The thread is detached: shutdown quits its loop and never joins it.
    pub(crate) fn start(shared: Arc<Shared>) -> std::io::Result<Self> {
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let (thread_context, thread_loop) = (context.clone(), main_loop.clone());
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        shared.stats.threads.fetch_add(1, Ordering::SeqCst);
        std::thread::Builder::new().name("update-pk".into()).spawn(move || {
            let _ = thread_context.with_thread_default(|| {
                ON_PK_THREAD.with(|flag| flag.set(true));
                let engine = Engine::new(shared);
                ENGINE.with(|cell| *cell.borrow_mut() = Some(engine));
                with(Engine::init);
                thread_loop.run();
                // Unsubscribe and drop the connection on this thread (a running transaction is packagekitd's).
                let engine = ENGINE.with(|cell| cell.borrow_mut().take());
                drop(engine);
            });
            let _ = done_tx.send(());
        })?;
        Ok(Self { context, main_loop, done: std::sync::Mutex::new(Some(done_rx)) })
    }
    /// Run `f` on the engine and wait for its answer (≤ 5 s). None: no answer (thread gone or stuck).
    pub(crate) fn ask<R: Send + 'static>(&self, f: impl FnOnce(&mut Engine) -> R + Send + 'static) -> Option<R> {
        if ON_PK_THREAD.with(|flag| flag.get()) {
            return None;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.context.invoke(move || {
            if let Some(result) = with(f) {
                let _ = tx.send(result);
            }
        });
        rx.recv_timeout(ASK_TIMEOUT).ok()
    }
    /// Quit the loop and wait at most `wait` for the thread to drop its engine (design §4.1: no join on
    /// shutdown, only this bounded wait). Its job lock is then free and a test bus connection closed, so a mirror
    /// that takes over next can reattach at once.
    pub(crate) fn stop(&self, wait: Duration) {
        let main_loop = self.main_loop.clone();
        self.context.invoke(move || main_loop.quit());
        if let Some(done) = self.done.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = done.recv_timeout(wait);
        }
    }
}

// ------------------------------------------------------------------------------------------- transactions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Refresh,
    GetUpdates,
    Details,
    UpdateDetail,
    Simulate,
    Update,
    Repair,
    OldTransactions,
}
#[derive(Debug)]
struct Tx {
    purpose: Purpose,
    subs: Vec<gio::SignalSubscriptionId>,
    packages: Vec<(u32, String, String)>,
    details: Vec<Value>,
    old: Vec<(String, bool, u32, String)>,
    error: Option<(u32, String)>,
    finished: Option<u32>,
    method_error: Option<bool>,
    status: u32,
    pct: Option<u32>,
    allow_cancel: bool,
    speed: u64,
    remaining: u32,
    download_left: Option<u64>,
    last_package: String,
    items: HashMap<String, (u32, u32)>,
    last_signal: Instant,
    ids: Vec<String>,
    /// status 30: unattended-upgrades holds the dpkg lock (`job.lockHolder`), read at most every [`HOLDER_RECHECK`]
    holder: bool,
    holder_checked: Option<Instant>,
}
impl Tx {
    fn new(purpose: Purpose, ids: Vec<String>) -> Self {
        Self {
            purpose,
            subs: Vec::new(),
            packages: Vec::new(),
            details: Vec::new(),
            old: Vec::new(),
            error: None,
            finished: None,
            method_error: None,
            status: 0,
            pct: None,
            allow_cancel: false,
            speed: 0,
            remaining: 0,
            download_left: None,
            last_package: String::new(),
            items: HashMap::new(),
            last_signal: Instant::now(),
            ids,
            holder: false,
            holder_checked: None,
        }
    }
    fn properties(&mut self, props: &glib::Variant) {
        let Some(map) = props.get::<HashMap<String, glib::Variant>>() else { return };
        for (key, value) in map {
            let value = unboxed(value);
            match key.as_str() {
                "Status" => self.status = value.get::<u32>().unwrap_or(self.status),
                "Percentage" => self.pct = value.get::<u32>().filter(|p| *p <= 100),
                "AllowCancel" => self.allow_cancel = value.get::<bool>().unwrap_or(false),
                "Speed" => self.speed = value.get::<u32>().map(u64::from).unwrap_or(0),
                "RemainingTime" => self.remaining = value.get::<u32>().unwrap_or(0),
                "DownloadSizeRemaining" => self.download_left = value.get::<u64>(),
                "LastPackage" => self.last_package = value.get::<String>().unwrap_or_default(),
                _ => {}
            }
        }
    }
}
/// The value inside a `v`, or the value itself. The values of an `a{sv}` read as `HashMap<String, Variant>` are
/// already unboxed, and `Variant::as_variant` on anything but a `v` is a GLib-CRITICAL (`g_variant_get_variant`).
pub(crate) fn unboxed(value: glib::Variant) -> glib::Variant {
    if value.is_type(glib::VariantTy::VARIANT) { value.as_variant().unwrap_or(value) } else { value }
}
/// A D-Bus error after which nobody knows what packagekitd did (the reply was lost).
fn uncertain(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::TimedOut)
        || error.matches(gio::IOErrorEnum::Closed)
        || ["NoReply", "Disconnected", "NameHasNoOwner", "ServiceUnknown"]
            .iter()
            .any(|name| error.message().contains(name))
}

// ------------------------------------------------------------------------------------------------- job

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Simulate,
    Confirm,
    Update,
    Repair,
    /// reattached to a transaction another owner started, or resolving one through GetOldTransactions
    Reattached,
}
struct Job {
    id: String,
    tid: Option<String>,
    role: &'static str,
    ids: Vec<String>,
    rows: Vec<String>,
    /// the chosen rows with their ids at the start (`model::row_info`), for the snapshot's `lastJob`
    row_info: Vec<Value>,
    started_at: u64,
    step: Step,
    installs: Vec<String>,
    removals: Vec<String>,
    download_size: Option<u64>,
    confirm: Option<Value>,
    committing: bool,
    status: u32,
    pct: Option<u32>,
    speed: u64,
    remaining: u32,
    download_left: Option<u64>,
    allow_cancel: bool,
    reattached: bool,
    states: HashMap<String, (&'static str, u32)>,
    restart: bool,
    written: Instant,
    _lock: Option<store::FileLock>,
}
impl Job {
    fn key(id: &str) -> String {
        id.splitn(4, ';').take(3).collect::<Vec<_>>().join(";")
    }
    fn row_states(&self, pkgs: &[Pkg]) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        for row in &self.rows {
            let members: Vec<&(&str, u32)> = pkgs
                .iter()
                .filter(|p| self.ids.contains(&p.id) && model::row_kind(&p.name).unwrap_or(if p.security() { "security" } else { "other" }) == row)
                .filter_map(|p| self.states.get(&Job::key(&p.id)))
                .collect();
            let mean = |state: &str| {
                let of: Vec<u32> = members.iter().filter(|(s, _)| *s == state).map(|(_, pct)| *pct).collect();
                (!of.is_empty()).then(|| of.iter().sum::<u32>() / of.len() as u32)
            };
            let (state, pct) = if !members.is_empty() && members.iter().all(|(s, _)| *s == "installed") {
                ("installed", 100)
            } else if let Some(pct) = mean("installing") {
                ("installing", pct)
            } else if let Some(pct) = mean("downloading") {
                ("downloading", pct)
            } else if self.committing {
                ("pendingInstall", 0)
            } else {
                ("pendingDownload", 0)
            };
            out.insert(row.clone(), json!({"state":state,"pct":pct}));
        }
        out
    }
    fn view(&self) -> Value {
        json!({"jobId":self.id,"role":self.role,"status":self.status,"pct":self.pct,"speed":self.speed,
               "remaining":self.remaining,"downloadLeft":self.download_left,"allowCancel":self.allow_cancel,
               "committing":self.committing,"startedAt":self.started_at,"reattached":self.reattached,
               "foreign":false,"confirm":self.confirm,"simulating":self.step == Step::Simulate,
               "downloadSize":self.download_size})
    }
}

// ------------------------------------------------------------------------------------------------ engine

/// Everything the owner knows; persisted to `snapshot.json` / `job.json` on each structural change.
pub(crate) struct Engine {
    shared: Arc<Shared>,
    conn: Option<gio::DBusConnection>,
    presence: Presence,
    subs: Vec<gio::SignalSubscriptionId>,
    sources: Vec<glib::Source>,
    progress_source: Option<glib::Source>,
    txs: HashMap<String, Tx>,
    job: Option<Job>,
    pkgs: Vec<Pkg>,
    checked_at: Option<u64>,
    checking: bool,
    refresh_error: Option<(u32, String)>,
    warnings: Vec<Value>,
    error: Option<Value>,
    repair_required: bool,
    foreign: Vec<String>,
    net: Net,
    scan: restart::Scan,
    scanned: Option<Instant>,
    require_restart: Option<Value>,
    download_size: Option<u64>,
    details: Lru,
    details_waiting: Vec<(Vec<String>, mpsc::SyncSender<ApiResult>)>,
    rev: u64,
    available: Option<usize>,
    vanished: bool,
    rate: Progress,
    /// the last job's record until the next check (rows `installed`, `restart`, `failed`; review F3)
    last_job: Option<Value>,
    /// who classified the current packages (`phasing::Classified`; review F5)
    phasing: Option<&'static str>,
    /// after a job the keep-alive lasts until the re-check answered, at most [`HOLD_AFTER_JOB`] (review F2)
    hold_after_check: bool,
    hold_timer: Option<glib::Source>,
    /// what the hero showed of our job at the last `persist()` ([`Engine::hero_key`]; review M1)
    hero: Option<HeroKey>,
    /// `UpdatesChanged` arrived at this time and the list was not read since; the debounce timer (review m2)
    changed_at: Option<Instant>,
    reread_timer: Option<glib::Source>,
    /// when the last GetUpdates started (a re-read after `UpdatesChanged` is not needed when one started later)
    list_started: Option<Instant>,
    // WU-FIX (GX1 review n5): the next rescan after a deferred receiver's `shell.restartResumed`
    restart_rescan: Option<glib::Source>,
}
/// What the page's hero shows of our running job (native/apps/settings/src/pages/update.rs `hero_model`): the stage,
/// `committing`, waiting for authentication (status 31; any other wait is the lock text) and whether
/// unattended-upgrades holds the lock.
type HeroKey = (model::Stage, bool, bool, bool);

fn lang_of(shared: &Shared) -> Lang {
    shared.ctx.lang()
}
fn err(code: &str, lang: Lang, key: &str) -> ApiError {
    ApiError::tr(code, lang, key, &json!({"product": crate::i18n::tr(lang, "update.product")}))
}
/// An error record as stored in the snapshot: codes, key and params, never rendered text (design §8).
fn record(code: &str, key: &str, detail: &str) -> Value {
    json!({"code":code,"key":key,"params":{},"detail":detail.chars().take(MAX_DETAIL).collect::<String>()})
}

impl Engine {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            conn: None,
            presence: Presence::Absent,
            subs: Vec::new(),
            sources: Vec::new(),
            progress_source: None,
            txs: HashMap::new(),
            job: None,
            pkgs: Vec::new(),
            checked_at: None,
            checking: false,
            refresh_error: None,
            warnings: Vec::new(),
            error: None,
            repair_required: false,
            foreign: Vec::new(),
            net: Net::default(),
            scan: restart::Scan::default(),
            scanned: None,
            require_restart: None,
            download_size: None,
            details: Lru::new(DETAILS_CACHE),
            details_waiting: Vec::new(),
            rev: 0,
            available: None,
            vanished: false,
            rate: Progress::default(),
            last_job: None,
            phasing: None,
            hold_after_check: false,
            hold_timer: None,
            hero: None,
            changed_at: None,
            reread_timer: None,
            list_started: None,
            // WU-FIX (GX1 review n5)
            restart_rescan: None,
        }
    }

    /// Connect, subscribe, load the last snapshot, reattach (design §4.7), scan, publish.
    fn init(&mut self) {
        self.load();
        if self.shared.setup.bus != BusTarget::Disabled {
            self.shared.stats.bus_opens.fetch_add(1, Ordering::SeqCst);
        }
        self.conn = connect(&self.shared.setup.bus);
        if let Some(conn) = self.conn.clone() {
            self.presence = presence(&conn);
            self.subscribe_manager(&conn);
            self.net = net::read(&conn);
            self.subs.push(net::subscribe(&conn, || {
                with(Engine::on_net);
            }));
        }
        self.watch_files();
        // WU-FIX (GX1 review n5)
        self.watch_restarts();
        let watchdog = glib::timeout_source_new(WATCHDOG, Some("update-watchdog"), glib::Priority::DEFAULT, || {
            with(Engine::on_watchdog);
            glib::ControlFlow::Continue
        });
        watchdog.attach(Some(&glib::MainContext::ref_thread_default()));
        self.sources.push(watchdog);
        self.rescan(true);
        self.reattach();
        self.persist();
    }
    /// Continue from the last owner's snapshot (`rev`, packages, sizes, restart record, check time).
    fn load(&mut self) {
        let Some(snapshot) = self.shared.store.read(SNAPSHOT) else { return };
        self.rev = snapshot["rev"].as_u64().unwrap_or(0);
        self.checked_at = snapshot["checkedAt"].as_u64();
        self.pkgs = snapshot["packages"].as_array().map(|a| a.iter().filter_map(Pkg::from_json).collect()).unwrap_or_default();
        self.warnings = snapshot["warnings"].as_array().cloned().unwrap_or_default();
        self.error = snapshot.get("error").filter(|e| e.is_object()).cloned();
        self.repair_required = snapshot["repairRequired"] == true;
        self.require_restart = snapshot.get("requireRestart").filter(|r| r.is_object()).cloned();
        self.download_size = snapshot["downloadSize"].as_u64();
        self.available = snapshot["totals"]["count"].as_u64().map(|n| n as usize);
        self.last_job = snapshot.get("lastJob").filter(|j| j.is_object()).cloned();
        self.phasing = match snapshot["phasing"].as_str() {
            Some("apt") => Some("apt"),
            Some("fallback") => Some("fallback"),
            Some("unknown") => Some("unknown"),
            _ => None,
        };
    }
    fn subscribe_manager(&mut self, conn: &gio::DBusConnection) {
        self.subs.push(conn.signal_subscribe(
            Some(SERVICE),
            Some(IFACE),
            None,
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            |_, _, _, _, member, params| {
                let member = member.to_owned();
                let params = params.clone();
                with(move |e| e.on_manager(&member, &params));
            },
        ));
        self.subs.push(conn.signal_subscribe(
            Some(DBUS),
            Some(DBUS),
            Some("NameOwnerChanged"),
            Some(DBUS_PATH),
            Some(SERVICE),
            gio::DBusSignalFlags::NONE,
            |_, _, _, _, _, params| {
                let owner = params.child_value(2).get::<String>().unwrap_or_default();
                with(move |e| e.on_owner(!owner.is_empty()));
            },
        ));
    }
    /// `/run/reboot-required` and apt's `history.log` (design §4.8): created, written or removed → rescan and
    /// publish. One inotify descriptor read on the pk context ([`restart::Watch`]; no GIO VFS, so no GVfs
    /// session-bus connection, review F9).
    fn watch_files(&mut self) {
        let paths = &self.shared.setup.paths;
        let Some(watch) = restart::Watch::new(&[paths.reboot_required.clone(), paths.apt_log.join("history.log")]) else { return };
        let source = glib::unix_fd_source_new(watch.fd(), glib::IOCondition::IN, Some("update-files"), glib::Priority::DEFAULT, move |_, condition| {
            if condition.intersects(glib::IOCondition::ERR | glib::IOCondition::HUP | glib::IOCondition::NVAL) {
                return glib::ControlFlow::Break;
            }
            if watch.changed() {
                with(|e| {
                    e.rescan(true);
                    e.persist();
                });
            }
            glib::ControlFlow::Continue
        });
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.sources.push(source);
    }

    // ---------------------------------------------------------------------------------- publish

    /// The persisted snapshot (design §4.5): codes, keys and params only.
    fn snapshot_json(&self) -> Value {
        let os = super::os_release(&self.shared.setup.paths);
        let backend = match self.presence {
            Presence::Absent => Value::Null,
            _ => json!("packagekit"),
        };
        json!({"version":1,"rev":self.rev,"bootId":store::boot_id(&self.shared.setup.paths),"checkedAt":self.checked_at,
               "checking":self.checking,"backend":backend,"os":{"version":model::os_version(&os)},
               "packages":self.pkgs.iter().map(Pkg::json).collect::<Vec<_>>(),"warnings":self.warnings,
               "error":self.error,"repairRequired":self.repair_required,"foreign":!self.foreign.is_empty(),
               "network":self.net.json(),"scan":self.scan.json(),"requireRestart":self.require_restart,
               "downloadSize":self.download_size,"totals":model::totals(&self.pkgs, self.download_size),
               "lastJob":self.last_job,"phasing":self.phasing,"owner":{"pid":std::process::id()}})
    }
    /// Write the snapshot with the next `rev`, then `update.changed` (and `system.updates.changed` when the
    /// non-phased count moved, design §4.4).
    fn persist(&mut self) {
        self.hero = self.hero_key();
        self.rev = store::next_rev(&self.shared.store, self.rev);
        self.shared.rev.store(self.rev, Ordering::SeqCst);
        let snapshot = self.snapshot_json();
        let _ = self.shared.store.write(SNAPSHOT, &snapshot);
        self.write_job();
        let view = super::compose(&self.shared, Some(&snapshot), self.shared.store.read(JOB).as_ref(), self.live().as_ref(), false);
        self.shared.ctx.emit(
            "update.changed",
            json!({"rev":view["rev"],"phase":view["phase"],"totals":view["totals"],"restart":view["restart"],
                   "job":view["job"],"error":view["error"],"release":{"state":view["release"]["state"],"version":view["release"]["version"]}}),
        );
        let count = view["totals"]["count"].as_u64().map(|n| n as usize);
        if count.is_some() && count != self.available && self.checked_at.is_some() {
            self.available = count;
            self.shared.ctx.emit(
                "system.updates.changed",
                json!({"available":count,"checkedAt":self.checked_at,"tool":"packagekit","security":view["totals"]["security"],
                       "checking":false,"rebootRequired":view["restart"]["reboot"]}),
            );
        }
    }
    /// The live job (progress fields the files do not carry at 4/s).
    pub(crate) fn live(&self) -> Option<Value> {
        let job = self.job.as_ref()?;
        let mut view = job.view();
        view["rows"] = json!(job.row_states(&self.pkgs));
        Some(view)
    }
    /// What the hero shows of our running job, as `compose` derives it (`model::stage` of status, committing and
    /// the simulate/confirm step). The page re-reads the snapshot only on `update.changed`, so a change here is
    /// structural: waiting → downloading → installing, lock → auth, `lockHolder` (review M1).
    fn hero_key(&self) -> Option<HeroKey> {
        let job = self.job.as_ref()?;
        let stage = model::stage(job.status, job.committing, job.step == Step::Simulate || job.confirm.is_some());
        let auth = stage == model::Stage::Waiting && job.status == model::status::WAITING_FOR_AUTH;
        let holder = job.tid.as_ref().and_then(|t| self.txs.get(t)).is_some_and(|tx| tx.holder);
        Some((stage, job.committing, auth, holder))
    }
    fn write_job(&mut self) {
        let boot = store::boot_id(&self.shared.setup.paths);
        let Some(job) = self.job.as_mut() else { return };
        let rows = job.row_states(&self.pkgs);
        let file = json!({"version":1,"jobId":job.id,"tid":job.tid,"bootId":boot,"role":job.role,"ids":job.ids,
            "rows":job.rows,"rowInfo":job.row_info,"startedAt":job.started_at,"owner":store::current_owner(),
            "simulate":{"installs":job.installs,"removals":job.removals,"downloadSize":job.download_size},
            "confirm":job.confirm,"committing":job.committing,"lastStatus":job.status,"pct":job.pct,
            "allowCancel":job.allow_cancel,"rowStates":rows,"reattached":job.reattached,
            "simulating":job.step == Step::Simulate});
        let _ = self.shared.store.write(JOB, &file);
        job.written = Instant::now();
    }
    fn progress(&mut self) {
        let now = Instant::now();
        let delivery = self.rate.offer(now);
        self.send_progress(delivery, now);
    }
    /// `now` is the throttle's own clock reading (the bus tests check the fan-out rate on it, review m4).
    fn send_progress(&mut self, delivery: Delivery, now: Instant) {
        if delivery == Delivery::None {
            return;
        }
        #[cfg(test)]
        if delivery == Delivery::Both {
            self.shared.stats.fanout.lock().unwrap_or_else(|e| e.into_inner()).push(now);
        }
        #[cfg(not(test))]
        let _ = now;
        let Some(view) = self.live() else { return };
        let event = json!({"jobId":view["jobId"],"status":view["status"],"pct":view["pct"],"speed":view["speed"],
                           "remaining":view["remaining"],"downloadLeft":view["downloadLeft"],
                           "allowCancel":view["allowCancel"],"rows":view["rows"]});
        if delivery == Delivery::Both {
            // emit() also dispatches to this host's own page (`model::Progress` counts it as a page event)
            self.shared.ctx.emit("update.progress", event);
        } else {
            self.shared.ctx.events.emit_local("update.progress", event);
        }
    }
    fn start_progress_timer(&mut self) {
        if self.progress_source.is_some() {
            return;
        }
        let source = glib::timeout_source_new(model::LOCAL_PROGRESS, Some("update-progress"), glib::Priority::DEFAULT, || {
            with(|e| {
                let now = Instant::now();
                let delivery = e.rate.tick(now);
                e.send_progress(delivery, now);
            });
            glib::ControlFlow::Continue
        });
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.progress_source = Some(source);
    }
    fn stop_progress_timer(&mut self) {
        // destroy(), not SourceId::remove(): that one only knows the global default context
        if let Some(source) = self.progress_source.take() {
            source.destroy();
        }
    }

    // ---------------------------------------------------------------------------------- API

    fn ready(&self, lang: Lang) -> Result<gio::DBusConnection, ApiError> {
        match (&self.conn, self.presence) {
            (Some(conn), Presence::Running | Presence::Activatable) => Ok(conn.clone()),
            _ => Err(err("unsupported", lang, "update.err.noPackageKit")),
        }
    }
    /// `update.check`: RefreshCache(false) → GetUpdates(none) → phasing → GetDetails (design §4.6).
    pub(crate) fn check(&mut self, lang: Lang) -> ApiResult {
        let conn = self.ready(lang)?;
        if self.job.is_some() {
            return Err(err("busy", lang, "update.err.busy"));
        }
        if self.checking {
            return Ok(json!({"started":true}));
        }
        if let Some(conn) = &self.conn {
            self.net = net::read(conn);
        }
        if self.net.refuses_check() {
            self.persist();
            return Err(err("offline", lang, "update.err.offline"));
        }
        self.checking = true;
        self.error = None;
        self.refresh_error = None;
        self.warnings.clear();
        // the last job's rows (installed, restart, failed) last until this check (review F3)
        self.last_job = None;
        self.start_tx(&conn, Purpose::Refresh, "RefreshCache", (false,).to_variant(), Vec::new());
        self.persist();
        Ok(json!({"started":true}))
    }
    /// `update.install {rows?}`: the ids of the current snapshot's rows, never phased or held ones.
    pub(crate) fn install(&mut self, rows: Option<Vec<String>>, lang: Lang) -> ApiResult {
        let conn = self.ready(lang)?;
        if self.job.is_some() || self.shared.store.locked(JOB_LOCK) {
            return Err(err("busy", lang, "update.err.busy"));
        }
        // WU-FIX (GW2b review n6): a running check would replace the ids under the job (review F16); its own text
        if self.checking {
            return Err(err("busy", lang, "update.err.checking"));
        }
        if !self.foreign.is_empty() {
            return Err(err("locked", lang, "update.err.locked"));
        }
        let os = model::os_version(&super::os_release(&self.shared.setup.paths));
        let grouped = model::group(&self.pkgs, &os);
        let ids = model::install_ids(&grouped, rows.as_deref()).ok_or_else(|| err("stale", lang, "update.err.stale"))?;
        let chosen: Vec<String> = match &rows {
            Some(rows) => rows.clone(),
            None => grouped.iter().map(|r| r.id.to_owned()).collect(),
        };
        let row_info: Vec<Value> = grouped.iter().filter(|r| chosen.iter().any(|c| c == r.id)).map(model::row_info).collect();
        let lock = self
            .shared
            .store
            .try_lock(JOB_LOCK)
            .ok()
            .flatten()
            .ok_or_else(|| err("busy", lang, "update.err.busy"))?;
        let id = store::token().map_err(|_| err("failed", lang, "update.err.failed"))?;
        self.job = Some(Job {
            id: id.clone(),
            tid: None,
            role: "update",
            ids: ids.clone(),
            rows: chosen,
            row_info,
            started_at: store::now_ms(),
            step: Step::Simulate,
            installs: Vec::new(),
            removals: Vec::new(),
            download_size: None,
            confirm: None,
            committing: false,
            status: 0,
            pct: None,
            speed: 0,
            remaining: 0,
            download_left: None,
            allow_cancel: false,
            reattached: false,
            states: HashMap::new(),
            restart: false,
            written: Instant::now(),
            _lock: Some(lock),
        });
        self.error = None;
        self.last_job = None;
        self.hold_app(true);
        self.start_progress_timer();
        // written before UpdatePackages (design §4.5)
        self.persist();
        self.start_tx(
            &conn,
            Purpose::Simulate,
            "UpdatePackages",
            (model::FLAG_SIMULATE | model::FLAG_ONLY_TRUSTED, ids.clone()).to_variant(),
            ids,
        );
        Ok(json!({"jobId":id}))
    }
    /// `update.confirm {jobId, accept}`: resume a job paused for removals or a metered download, or end it.
    pub(crate) fn confirm(&mut self, job_id: &str, accept: bool, lang: Lang) -> ApiResult {
        let conn = self.ready(lang)?;
        let Some(job) = self.job.as_mut().filter(|j| j.id == job_id && j.step == Step::Confirm) else {
            return Err(err("stale", lang, "update.err.stale"));
        };
        job.confirm = None;
        if !accept {
            self.finish_job(json!({"exit":model::exit::CANCELLED,"code":"declined","key":null,"params":{},"detail":""}), false);
            return Ok(json!({"ok":true}));
        }
        job.step = Step::Update;
        let ids = job.ids.clone();
        // a transaction runs again: keep the process when the window closes (review F7)
        self.hold_app(true);
        self.start_tx(&conn, Purpose::Update, "UpdatePackages", (model::FLAG_ONLY_TRUSTED, ids.clone()).to_variant(), ids);
        self.persist();
        Ok(json!({"ok":true}))
    }
    /// `update.cancel {jobId}`: only while PackageKit allows it (download phase).
    pub(crate) fn cancel(&mut self, job_id: &str, lang: Lang) -> ApiResult {
        let conn = self.ready(lang)?;
        let Some(job) = self.job.as_ref().filter(|j| j.id == job_id) else {
            return Err(err("stale", lang, "update.err.stale"));
        };
        let tid = job.tid.clone();
        let allowed = tid.as_ref().and_then(|t| self.txs.get(t)).is_some_and(|tx| tx.allow_cancel);
        match tid {
            Some(tid) if allowed => {
                conn.call(Some(SERVICE), &tid, TX_IFACE, "Cancel", None, None, gio::DBusCallFlags::NONE, 10_000, gio::Cancellable::NONE, |_| {});
                Ok(json!({"ok":true}))
            }
            _ => Err(err("cannot_cancel", lang, "update.err.cannotCancel")),
        }
    }
    /// `update.repair`: RepairSystem(ONLY_TRUSTED), only in repairRequired (polkit asks a password).
    pub(crate) fn repair(&mut self, lang: Lang) -> ApiResult {
        let conn = self.ready(lang)?;
        if self.job.is_some() || self.shared.store.locked(JOB_LOCK) {
            return Err(err("busy", lang, "update.err.busy"));
        }
        if !self.repair_required {
            return Err(err("stale", lang, "update.err.stale"));
        }
        let lock = self.shared.store.try_lock(JOB_LOCK).ok().flatten().ok_or_else(|| err("busy", lang, "update.err.busy"))?;
        let id = store::token().map_err(|_| err("failed", lang, "update.err.failed"))?;
        self.job = Some(Job {
            id: id.clone(),
            tid: None,
            role: "repair",
            ids: Vec::new(),
            rows: Vec::new(),
            row_info: Vec::new(),
            started_at: store::now_ms(),
            step: Step::Repair,
            installs: Vec::new(),
            removals: Vec::new(),
            download_size: None,
            confirm: None,
            committing: true,
            status: 0,
            pct: None,
            speed: 0,
            remaining: 0,
            download_left: None,
            allow_cancel: false,
            reattached: false,
            states: HashMap::new(),
            restart: false,
            written: Instant::now(),
            _lock: Some(lock),
        });
        self.last_job = None;
        self.hold_app(true);
        self.start_progress_timer();
        self.persist();
        self.start_tx(&conn, Purpose::Repair, "RepairSystem", (model::FLAG_ONLY_TRUSTED,).to_variant(), Vec::new());
        Ok(json!({"jobId":id}))
    }
    /// `update.details {ids}`: cached per id; otherwise one GetUpdateDetail whose answer goes to `reply`.
    pub(crate) fn details(&mut self, ids: Vec<String>, lang: Lang, reply: mpsc::SyncSender<ApiResult>) {
        let known: Vec<String> = ids.into_iter().filter(|id| self.pkgs.iter().any(|p| &p.id == id)).take(DETAILS_PER_CALL).collect();
        if known.is_empty() {
            let _ = reply.send(Err(err("stale", lang, "update.err.stale")));
            return;
        }
        let missing: Vec<String> = known.iter().filter(|id| !self.details.contains(id)).cloned().collect();
        if missing.is_empty() {
            let _ = reply.send(Ok(json!({"details":self.cached_details(&known)})));
            return;
        }
        let conn = match self.ready(lang) {
            Ok(conn) => conn,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        self.details_waiting.push((known, reply));
        self.start_tx(&conn, Purpose::UpdateDetail, "GetUpdateDetail", (missing.clone(),).to_variant(), missing);
    }
    pub(crate) fn cached_details(&mut self, ids: &[String]) -> Vec<Value> {
        ids.iter().filter_map(|id| self.details.get(id)).collect()
    }

    // ---------------------------------------------------------------------------------- transactions

    fn start_tx(&mut self, conn: &gio::DBusConnection, purpose: Purpose, method: &'static str, args: glib::Variant, ids: Vec<String>) {
        if purpose == Purpose::GetUpdates {
            self.list_started = Some(Instant::now());
        }
        conn.call(
            Some(SERVICE),
            PATH,
            IFACE,
            "CreateTransaction",
            None,
            None,
            gio::DBusCallFlags::NONE,
            10_000,
            gio::Cancellable::NONE,
            move |reply| {
                with(move |e| e.on_created(reply, purpose, method, args, ids));
            },
        );
    }
    fn on_created(&mut self, reply: Result<glib::Variant, glib::Error>, purpose: Purpose, method: &'static str, args: glib::Variant, ids: Vec<String>) {
        let Some(conn) = self.conn.clone() else { return };
        let path = reply
            .ok()
            .and_then(|v| v.child_value(0).str().map(str::to_owned))
            .filter(|p| p.starts_with('/') && p.len() <= 512);
        let Some(path) = path else {
            self.tx_failed(purpose, None, true);
            return;
        };
        let mut tx = Tx::new(purpose, ids);
        tx.subs = self.subscribe_tx(&conn, &path);
        self.txs.insert(path.clone(), tx);
        if matches!(purpose, Purpose::Simulate | Purpose::Update | Purpose::Repair)
            && let Some(job) = self.job.as_mut()
        {
            job.tid = Some(path.clone());
            self.write_job();
        }
        let hints = self.hints();
        conn.call(Some(SERVICE), &path, TX_IFACE, "SetHints", Some(&(hints,).to_variant()), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |_| {});
        let tid = path.clone();
        conn.call(Some(SERVICE), &path, TX_IFACE, method, Some(&args), None, gio::DBusCallFlags::NONE, i32::MAX, gio::Cancellable::NONE, move |reply| {
            if let Err(error) = reply {
                let unknown = uncertain(&error);
                with(move |e| {
                    if let Some(tx) = e.txs.get_mut(&tid) {
                        tx.method_error = Some(unknown);
                    }
                    e.end_tx(&tid);
                });
            }
        });
    }
    fn subscribe_tx(&self, conn: &gio::DBusConnection, path: &str) -> Vec<gio::SignalSubscriptionId> {
        vec![
            conn.signal_subscribe(Some(SERVICE), Some(TX_IFACE), None, Some(path), None, gio::DBusSignalFlags::NONE, |_, _, path, _, member, params| {
                let (path, member, params) = (path.to_owned(), member.to_owned(), params.clone());
                with(move |e| e.on_tx_signal(&path, &member, &params));
            }),
            conn.signal_subscribe(Some(SERVICE), Some(PROPERTIES), Some("PropertiesChanged"), Some(path), None, gio::DBusSignalFlags::NONE, |_, _, path, _, _, params| {
                if params.child_value(0).str() == Some(TX_IFACE) {
                    let (path, props) = (path.to_owned(), params.child_value(1));
                    with(move |e| e.on_tx_properties(&path, &props));
                }
            }),
        ]
    }
    /// `interactive=true` (polkit), `background=false`, a locale only when `locale -a` lists it (design §4.6).
    fn hints(&self) -> Vec<String> {
        let mut hints = vec!["interactive=true".to_owned(), "background=false".to_owned()];
        let wanted = if lang_of(&self.shared) == Lang::En { "en_GB.UTF-8" } else { "nl_NL.UTF-8" };
        if super::locale_available(wanted) {
            hints.push(format!("locale={wanted}"));
        }
        hints
    }
    fn on_tx_properties(&mut self, path: &str, props: &glib::Variant) {
        let Some(tx) = self.txs.get_mut(path) else { return };
        tx.last_signal = Instant::now();
        tx.properties(props);
        self.job_progress(path);
    }
    fn on_tx_signal(&mut self, path: &str, member: &str, params: &glib::Variant) {
        let Some(tx) = self.txs.get_mut(path) else { return };
        tx.last_signal = Instant::now();
        match member {
            "Package" => {
                if let Some(p) = params.get::<(u32, String, String)>()
                    && tx.packages.len() < MAX_PACKAGES
                {
                    tx.packages.push(p);
                }
            }
            "Packages" => {
                if let Some((list,)) = params.get::<(Vec<(u32, String, String)>,)>() {
                    let room = MAX_PACKAGES.saturating_sub(tx.packages.len());
                    tx.packages.extend(list.into_iter().take(room));
                }
            }
            "ItemProgress" => {
                if let Some((id, status, pct)) = params.get::<(String, u32, u32)>() {
                    tx.items.insert(Job::key(&id), (status, pct.min(100)));
                }
            }
            "Details" => {
                if let Some((map,)) = params.get::<(HashMap<String, glib::Variant>,)>() {
                    let id = map.get("package-id").and_then(|v| v.get::<String>());
                    let size = map.get("download-size").and_then(|v| v.get::<u64>());
                    if let Some(id) = id {
                        tx.details.push(json!({"id":id,"size":size}));
                    }
                }
            }
            "UpdateDetail" => {
                if let Some(d) = update_detail(params) {
                    tx.details.push(d);
                }
            }
            "UpdateDetails" => {
                let list = params.child_value(0);
                for i in 0..list.n_children().min(MAX_PACKAGES) {
                    if let Some(d) = update_detail(&list.child_value(i)) {
                        tx.details.push(d);
                    }
                }
            }
            "RequireRestart" => {
                if let Some((kind, id)) = params.get::<(u32, String)>()
                    && model::RESTART_TYPES.contains(&kind)
                {
                    let boot = store::boot_id(&self.shared.setup.paths);
                    // merge only with a record of this boot; a reboot (system 4, security-system 6) outranks a
                    // new session (3, 5)
                    let current = self.require_restart.as_ref().filter(|r| boot.is_some() && r["bootId"].as_str() == boot.as_deref());
                    let mut pkgs: Vec<Value> = current.and_then(|r| r["pkgs"].as_array().cloned()).unwrap_or_default();
                    let name = id.split(';').next().unwrap_or("").to_owned();
                    if !pkgs.iter().any(|p| p == &json!(name)) && pkgs.len() < 50 {
                        pkgs.push(json!(name));
                    }
                    let rank = |t: u32| if [4, 6].contains(&t) { 2 } else { 1 };
                    let previous = current.and_then(|r| r["type"].as_u64()).map(|t| t as u32);
                    let kind = match previous {
                        Some(p) if rank(p) >= rank(kind) => p,
                        _ => kind,
                    };
                    self.require_restart = Some(json!({"type":kind,"pkgs":pkgs,"bootId":boot}));
                    if let Some(job) = self.job.as_mut() {
                        job.restart = true;
                    }
                }
            }
            "ErrorCode" => {
                if let Some((code, detail)) = params.get::<(u32, String)>() {
                    tx.error = Some((code, detail.chars().take(MAX_DETAIL).collect()));
                }
            }
            "Transaction" => {
                if let Some((object, _, succeeded, role, _, data, _, _)) =
                    params.get::<(glib::variant::ObjectPath, String, bool, u32, u32, String, u32, String)>()
                {
                    tx.old.push((object.as_str().to_owned(), succeeded, role, data));
                }
            }
            "Finished" => {
                if let Some((exit, _)) = params.get::<(u32, u32)>() {
                    tx.finished = Some(exit);
                    let path = path.to_owned();
                    self.end_tx(&path);
                }
                return;
            }
            "Destroy" => {
                let path = path.to_owned();
                self.end_tx(&path);
                return;
            }
            _ => {}
        }
        self.job_progress(path);
    }

    /// Fold a transaction's progress into the job (statuses, committing, per-package states) and throttle.
    fn job_progress(&mut self, path: &str) {
        let Some(tx) = self.txs.get_mut(path) else { return };
        if !matches!(tx.purpose, Purpose::Simulate | Purpose::Update | Purpose::Repair) {
            return;
        }
        let Some(job) = self.job.as_mut().filter(|j| j.tid.as_deref() == Some(path)) else { return };
        // who holds the dpkg lock we wait for (`job.lockHolder`): a /proc scan, at most every 2 s
        if tx.status == model::status::WAITING_FOR_LOCK {
            if tx.holder_checked.is_none_or(|at| at.elapsed() >= HOLDER_RECHECK) {
                tx.holder = guard::unattended_running(&self.shared.setup.paths);
                tx.holder_checked = Some(Instant::now());
            }
        } else {
            tx.holder = false;
            tx.holder_checked = None;
        }
        let tx = &*tx;
        let status_changed = job.status != tx.status;
        let was_cancellable = job.allow_cancel;
        job.status = tx.status;
        job.pct = tx.pct;
        job.speed = tx.speed;
        job.remaining = tx.remaining;
        job.download_left = tx.download_left;
        job.allow_cancel = tx.allow_cancel;
        // totals.downloadSize during the download (GW3 D1): the simulate's DownloadSizeRemaining, or else the
        // first one the real download reports (the apt backend may leave the simulate's at 0)
        let mut filled = false;
        if tx.purpose == Purpose::Update
            && job.download_size.is_none_or(|size| size == 0)
            && let Some(left) = tx.download_left.filter(|left| *left > 0 && model::status::DOWNLOADING.contains(&tx.status))
        {
            job.download_size = Some(left);
            self.download_size = Some(left);
            // the page read the snapshot before this total existed (review m1)
            filled = true;
        }
        if tx.purpose != Purpose::Simulate
            && (model::status::COMMITTING.contains(&tx.status) || (was_cancellable && !tx.allow_cancel && job.step == Step::Update && tx.status != 0 && !model::status::DOWNLOADING.contains(&tx.status)))
        {
            job.committing = true;
        }
        if tx.purpose == Purpose::Update {
            for (info, id, _) in &tx.packages {
                let state = match *info {
                    model::info::DOWNLOADING => "downloading",
                    model::info::UPDATING | model::info::INSTALLING => "installing",
                    model::info::FINISHED => "installed",
                    _ => continue,
                };
                let entry = job.states.entry(Job::key(id)).or_insert((state, 0));
                if entry.0 != "installed" {
                    *entry = (state, entry.1);
                }
            }
            for (key, (status, pct)) in &tx.items {
                let state = match *status {
                    8 => "downloading",
                    9 | 10 | 6 | 12 => "installing",
                    18 => "installed",
                    _ => continue,
                };
                job.states.insert(key.clone(), (state, if state == "installed" { 100 } else { *pct }));
            }
        }
        let rewrite = status_changed || job.written.elapsed() >= JOB_REWRITE;
        // a new stage, wait kind or lock holder, or the download total: update.changed, so the page re-reads the
        // snapshot (its hero switches on `phase`; update.progress only patches the numbers; review M1, m1)
        if filled || self.hero_key() != self.hero {
            self.persist();
        } else if rewrite {
            self.write_job();
        }
        self.progress();
    }

    fn end_tx(&mut self, path: &str) {
        let Some(mut tx) = self.txs.remove(path) else { return };
        if let Some(conn) = &self.conn {
            for sub in tx.subs.drain(..) {
                conn.signal_unsubscribe(sub);
            }
        }
        if tx.finished.is_none() && tx.method_error.is_none() {
            // Destroy without Finished, or the daemon left: nobody knows the outcome
            self.tx_failed(tx.purpose, None, true);
            return;
        }
        if let Some(unknown) = tx.method_error
            && tx.finished.is_none()
        {
            self.tx_failed(tx.purpose, tx.error.clone(), unknown);
            return;
        }
        let exit = tx.finished.unwrap_or(model::exit::FAILED);
        match tx.purpose {
            Purpose::Refresh => {
                self.refresh_error = model::map_failure(tx.error.as_ref().map(|e| e.0), exit).and(tx.error.clone());
                if let Some(conn) = self.conn.clone() {
                    self.start_tx(&conn, Purpose::GetUpdates, "GetUpdates", (model::FILTER_NONE,).to_variant(), Vec::new());
                }
            }
            Purpose::GetUpdates => self.on_updates(tx, exit),
            Purpose::Details => {
                for d in &tx.details {
                    if let (Some(id), Some(size)) = (d["id"].as_str(), d["size"].as_u64())
                        && let Some(p) = self.pkgs.iter_mut().find(|p| p.id == id)
                    {
                        p.size = Some(size);
                    }
                }
                self.finish_check();
            }
            Purpose::UpdateDetail => {
                for d in tx.details {
                    // only ids of the current snapshot (the cache is cleared on each GetUpdates)
                    if let Some(id) = d["id"].as_str().filter(|id| self.pkgs.iter().any(|p| p.id == *id)) {
                        self.details.insert(id.to_owned(), d.clone());
                    }
                }
                let failed = model::map_failure(tx.error.as_ref().map(|e| e.0), exit).is_some();
                let lang = lang_of(&self.shared);
                for (ids, reply) in std::mem::take(&mut self.details_waiting) {
                    let found = self.cached_details(&ids);
                    let _ = reply.send(if found.is_empty() && failed {
                        Err(err("details", lang, "update.err.details"))
                    } else {
                        Ok(json!({"details":found}))
                    });
                }
            }
            Purpose::Simulate => self.on_simulated(tx, exit),
            Purpose::Update | Purpose::Repair => {
                let failure = model::map_failure(tx.error.as_ref().map(|e| e.0), exit);
                let detail = tx.error.as_ref().map(|e| e.1.clone()).unwrap_or_default();
                let outcome = match failure {
                    None => json!({"exit":model::exit::SUCCESS,"code":null,"key":null,"params":{},"detail":""}),
                    Some(f) => json!({"exit":exit,"code":f.code,"key":f.key,"params":{},"detail":detail}),
                };
                if failure.is_some_and(|f| f.code == "repair_required") {
                    self.repair_required = true;
                }
                // a success (ours, of RepairSystem or of UpdatePackages) clears repairRequired in finish_job
                self.finish_job(outcome, true);
            }
            Purpose::OldTransactions => self.on_old_transactions(tx),
        }
    }
    /// A transaction that could not start or whose outcome is unknown.
    fn tx_failed(&mut self, purpose: Purpose, error: Option<(u32, String)>, unknown: bool) {
        let detail = error.as_ref().map(|e| e.1.clone()).unwrap_or_default();
        match purpose {
            Purpose::Refresh | Purpose::GetUpdates | Purpose::Details => {
                self.checked();
                if purpose != Purpose::Details {
                    self.error = Some(record("check_failed", "update.err.checkFailed", &detail));
                }
                self.persist();
            }
            Purpose::UpdateDetail => {
                let lang = lang_of(&self.shared);
                for (_, reply) in std::mem::take(&mut self.details_waiting) {
                    let _ = reply.send(Err(err("details", lang, "update.err.details")));
                }
            }
            Purpose::Simulate | Purpose::Update | Purpose::Repair => {
                if unknown && purpose != Purpose::Simulate {
                    // resolved by GetOldTransactions (design §4.6: "a vanished owner gives outcome unknown,
                    // resolved by GetOldTransactions when the daemon is back"; the new transaction activates it)
                    self.vanished = true;
                    let tid = self.job.as_ref().and_then(|j| j.tid.clone());
                    if let (Some(conn), Some(tid)) = (self.conn.clone(), tid) {
                        if let Some(job) = self.job.as_mut() {
                            job.step = Step::Reattached;
                        }
                        self.persist();
                        self.start_tx(&conn, Purpose::OldTransactions, "GetOldTransactions", (5u32,).to_variant(), vec![tid]);
                        return;
                    }
                    let outcome = json!({"exit":null,"code":"unknown_outcome","key":"update.err.unknownOutcome","params":{},"detail":detail});
                    self.finish_job(outcome, true);
                } else {
                    let f = model::map_failure(error.as_ref().map(|e| e.0), model::exit::FAILED).unwrap_or(model::Failure { code: "failed", key: "update.err.failed" });
                    self.finish_job(json!({"exit":model::exit::FAILED,"code":f.code,"key":f.key,"params":{},"detail":detail}), true);
                }
            }
            Purpose::OldTransactions => {
                self.finish_job(json!({"exit":null,"code":"unknown_outcome","key":"update.err.unknownOutcome","params":{},"detail":""}), true);
            }
        }
    }
    fn on_updates(&mut self, tx: Tx, exit: u32) {
        let failure = model::map_failure(tx.error.as_ref().map(|e| e.0), exit);
        if failure.is_some() {
            let network = self.refresh_error.as_ref().is_some_and(|e| model::NETWORK_ERRORS.contains(&e.0))
                || tx.error.as_ref().is_some_and(|e| model::NETWORK_ERRORS.contains(&e.0));
            let detail = tx.error.map(|e| e.1).unwrap_or_default();
            self.error = Some(if network {
                record("network", "update.err.network", &detail)
            } else {
                record("check_failed", "update.err.checkFailed", &detail)
            });
            self.checked();
            self.persist();
            return;
        }
        // a refresh error with a working GetUpdates: a warning for a non-Ubuntu source, else the network error
        if let Some((code, detail)) = self.refresh_error.take() {
            match model::source_host(&detail).filter(|_| model::NETWORK_ERRORS.contains(&code)) {
                Some(host) => self.warnings.push(json!({"code":"source","key":"update.warn.source","params":{"source":host}})),
                None if model::NETWORK_ERRORS.contains(&code) => self.error = Some(record("network", "update.err.network", &detail)),
                None => {}
            }
        }
        let old: HashMap<String, Option<u64>> = self.pkgs.iter().map(|p| (p.id.clone(), p.size)).collect();
        let mut pkgs: Vec<Pkg> = tx
            .packages
            .iter()
            .filter(|(info, _, _)| *info != 1)
            .filter_map(|(info, id, summary)| Pkg::from_signal(*info, id, summary))
            .collect();
        for p in pkgs.iter_mut() {
            p.size = old.get(&p.id).copied().flatten();
        }
        // a new list: details of the old one are not kept (review F15)
        self.details.clear();
        // phasing runs off the pk thread (1–2 s of apt cache), the result comes back through the context
        let tools = self.shared.setup.tools.clone();
        let context = glib::MainContext::ref_thread_default();
        let spawned = std::thread::Builder::new().name("update-phasing".into()).spawn(move || {
            let mut pkgs = pkgs;
            let classified = phasing::classify(&tools, &mut pkgs).map(phasing::Classified::as_str).map_err(|_| ());
            context.invoke(move || {
                with(move |e| e.on_classified(pkgs, classified));
            });
        });
        if spawned.is_err() {
            self.checked();
            self.persist();
        }
    }
    /// `classified`: who classified (`apt`, `fallback`, `unknown`), or Err for a missing stub (test mode:
    /// `test_seam`, review F5). Unclassified packages already carry `phasing::unknown`'s safe rule.
    fn on_classified(&mut self, pkgs: Vec<Pkg>, classified: Result<&'static str, ()>) {
        let job_ids: Vec<String> = self.job.as_ref().map(|j| j.ids.clone()).unwrap_or_default();
        self.pkgs = pkgs;
        self.phasing = Some(classified.unwrap_or("unknown"));
        if classified.is_err() {
            self.error = Some(record("test_seam", "update.err.testSeam", ""));
        }
        self.checked_at = Some(store::now_ms());
        // sizes for the rows: one GetDetails (the apt backend very likely has no download-size, §3.3)
        let ids: Vec<String> = self.pkgs.iter().filter(|p| p.offered() && p.size.is_none()).map(|p| p.id.clone()).take(500).collect();
        if !ids.is_empty()
            && job_ids.is_empty()
            && let Some(conn) = self.conn.clone()
        {
            self.start_tx(&conn, Purpose::Details, "GetDetails", (ids.clone(),).to_variant(), ids);
            return;
        }
        self.finish_check();
    }
    fn finish_check(&mut self) {
        self.checked();
        self.rescan(true);
        self.persist();
    }
    /// A check (the user's or a job's re-check) ended: the keep-alive of a finished job may go (review F2).
    fn checked(&mut self) {
        self.checking = false;
        if self.hold_after_check {
            self.release_hold();
        }
    }
    /// The keep-alive after a job: until the re-check answered, at most [`HOLD_AFTER_JOB`] (review F2).
    fn hold_until_checked(&mut self) {
        self.hold_after_check = true;
        let source = glib::timeout_source_new(HOLD_AFTER_JOB, Some("update-hold"), glib::Priority::DEFAULT, || {
            with(Engine::release_hold);
            glib::ControlFlow::Break
        });
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        if let Some(old) = self.hold_timer.replace(source) {
            old.destroy();
        }
    }
    fn release_hold(&mut self) {
        self.hold_after_check = false;
        if let Some(source) = self.hold_timer.take() {
            source.destroy();
        }
        // a new job took the hold in the meantime: it releases it itself
        if self.job.is_none() {
            self.hold_app(false);
        }
    }
    fn on_simulated(&mut self, tx: Tx, exit: u32) {
        let Some(job) = self.job.as_mut() else { return };
        if let Some(f) = model::map_failure(tx.error.as_ref().map(|e| e.0), exit) {
            let detail = tx.error.map(|e| e.1).unwrap_or_default();
            self.finish_job(json!({"exit":exit,"code":f.code,"key":f.key,"params":{},"detail":detail}), true);
            return;
        }
        job.installs = tx.packages.iter().filter(|(info, _, _)| [11, 12].contains(info)).map(|(_, id, _)| id.clone()).take(2000).collect();
        job.removals = tx
            .packages
            .iter()
            .filter(|(info, _, _)| model::info::CONFIRM.contains(info))
            .map(|(_, id, _)| id.split(';').next().unwrap_or("").to_owned())
            .take(200)
            .collect();
        // 0 is "unknown" (the apt backend may leave it at 0): the download fills it in (review m1)
        job.download_size = tx.download_left.filter(|size| *size > 0);
        let metered_ok = self.shared.ctx.settings()["update"]["metered"] == true;
        if !job.removals.is_empty() {
            job.confirm = Some(json!({"kind":"removals","removals":job.removals}));
        } else if self.net.metered && !metered_ok {
            job.confirm = Some(json!({"kind":"metered","size":job.download_size}));
        }
        self.download_size = job.download_size;
        if job.confirm.is_some() {
            job.step = Step::Confirm;
            // no transaction runs while the user decides: closing the window may end the process (review F7);
            // a new owner then ends the paused job as declined (model::Reattach::Expired)
            self.hold_app(false);
            self.persist();
            return;
        }
        job.step = Step::Update;
        let ids = job.ids.clone();
        if let Some(conn) = self.conn.clone() {
            self.start_tx(&conn, Purpose::Update, "UpdatePackages", (model::FLAG_ONLY_TRUSTED, ids.clone()).to_variant(), ids);
        }
        self.persist();
    }
    /// End our job: outcome into `job.json` and `jobs.jsonl`, release the lock, re-check (design §4.6 "After
    /// Finished"), notify, release the keep-alive once the re-check answered (review F2).
    ///
    /// On success the job's packages leave the list at once, so a snapshot written before the re-check answers
    /// (or by a process that exits before it) never offers them again; the snapshot's `lastJob` keeps the job's
    /// rows, from which `update.status` derives `installed`, `restart` and `failed` until the next check (F3).
    fn finish_job(&mut self, outcome: Value, recheck: bool) {
        let Some(mut job) = self.job.take() else { return };
        let now = store::now_ms();
        let success = outcome["code"].is_null() && outcome["exit"] == model::exit::SUCCESS;
        let failed = outcome["code"].is_string() && outcome["code"] != "declined";
        if failed && outcome["code"] != "cancelled" {
            self.error = Some(json!({"code":outcome["code"],"key":outcome["key"],"params":{},"detail":outcome["detail"]}));
        } else if outcome["code"] == "cancelled" {
            self.warnings.push(json!({"code":"cancelled","key":"update.err.cancelled","params":{}}));
        }
        if success {
            // a finished RepairSystem or UpdatePackages: dpkg is consistent again (review F14), and the last outcome
            // is no longer a failure (design §4.5 `error`: "until the next check or success")
            self.repair_required = false;
            self.error = None;
            self.pkgs.retain(|p| !job.ids.contains(&p.id));
        }
        // this job's download total is over; the next simulate reports its own
        self.download_size = None;
        if job.role == "update" {
            self.last_job = Some(json!({"jobId":job.id,"role":job.role,"finishedAt":now,"outcome":outcome,"rows":job.row_info}));
        }
        let boot = store::boot_id(&self.shared.setup.paths);
        let mut file = self.shared.store.read(JOB).unwrap_or_else(|| json!({}));
        file["version"] = json!(1);
        file["jobId"] = json!(job.id);
        file["role"] = json!(job.role);
        file["bootId"] = json!(boot);
        file["committing"] = json!(job.committing);
        file["finishedAt"] = json!(now);
        file["outcome"] = outcome.clone();
        file["seen"] = json!(false);
        let _ = self.shared.store.write(JOB, &file);
        let _ = self.shared.store.append_job(&json!({"jobId":job.id,"role":job.role,"startedAt":job.started_at,"finishedAt":now,"outcome":outcome}), now);
        job._lock.take();
        self.stop_progress_timer();
        let restart = job.restart;
        let mut rechecking = false;
        if recheck && outcome["code"] != "unknown_outcome" && let Some(conn) = self.conn.clone() {
            self.checking = true;
            self.start_tx(&conn, Purpose::GetUpdates, "GetUpdates", (model::FILTER_NONE,).to_variant(), Vec::new());
            rechecking = true;
        }
        self.rescan(true);
        self.persist();
        // a declined confirm or a cancel is the user's own choice: no notification
        if !matches!(outcome["code"].as_str(), Some("declined" | "cancelled")) {
            self.notify(failed, restart);
        }
        if rechecking {
            self.hold_until_checked();
        } else {
            self.hold_app(false);
        }
    }
    /// The design's notifications (§3.8), only from the owner: failed / installed when no page listens,
    /// restart required when `settings.update.notifyRestart` is on. "No page listens": the native Settings said it
    /// has no window (`update.setWatching`; its stream stays open while its keep-alive holds the process), else no
    /// event stream (ServiceContext::watching).
    fn notify(&self, failed: bool, restart: bool) {
        if self.shared.ctx.dry_run {
            return;
        }
        let lang = lang_of(&self.shared);
        let watching = self.shared.ctx.watching();
        let notify_restart = self.shared.ctx.settings()["update"]["notifyRestart"] != false;
        let (title, body) = if failed {
            if watching {
                return;
            }
            (crate::i18n::tr(lang, "update.notify.failed"), String::new())
        } else if restart && notify_restart {
            (crate::i18n::tr(lang, "update.notify.restart"), crate::i18n::tr(lang, "update.notify.restartBody"))
        } else if !watching {
            (crate::i18n::tr(lang, "update.notify.installed"), crate::i18n::tr(lang, "update.notify.installedBody"))
        } else {
            return;
        };
        super::notify(&self.shared, &title, &body);
    }
    /// Keep the Settings process while our job runs (design §4.1 C2): closing the window then only drops
    /// the window, `Application::hold()` keeps the process until `Finished`.
    fn hold_app(&self, on: bool) {
        if self.shared.stats.held.swap(on, Ordering::SeqCst) != on {
            super::hold_application(on);
        }
    }

    // ---------------------------------------------------------------------------------- reattach (§4.7)

    fn reattach(&mut self) {
        let boot = store::boot_id(&self.shared.setup.paths);
        let job = self.shared.store.read(JOB);
        let plan = model::reattach(job.as_ref(), boot.as_deref());
        let Some(job) = job else { return };
        if plan == model::Reattach::Nothing {
            return;
        }
        let expired = plan == model::Reattach::Expired;
        let conn = self.conn.clone();
        if conn.is_none() && !expired {
            return;
        }
        let lock = self.shared.store.try_lock(JOB_LOCK).ok().flatten();
        let mut reattached = Job {
            id: job["jobId"].as_str().filter(|t| store::is_token(t)).unwrap_or("").to_owned(),
            tid: None,
            role: if job["role"] == "repair" { "repair" } else { "update" },
            ids: strings(&job["ids"]),
            rows: strings(&job["rows"]),
            row_info: job["rowInfo"].as_array().cloned().unwrap_or_default(),
            started_at: job["startedAt"].as_u64().unwrap_or(0),
            step: Step::Reattached,
            installs: strings(&job["simulate"]["installs"]),
            removals: strings(&job["simulate"]["removals"]),
            download_size: job["simulate"]["downloadSize"].as_u64(),
            confirm: None,
            committing: job["committing"] == true,
            status: job["lastStatus"].as_u64().unwrap_or(0) as u32,
            pct: None,
            speed: 0,
            remaining: 0,
            download_left: None,
            allow_cancel: false,
            reattached: true,
            states: HashMap::new(),
            restart: false,
            written: Instant::now(),
            _lock: lock,
        };
        // a job that waited for a confirm: only the simulate ran, so nothing was installed (review F7)
        let Some(conn) = conn.filter(|_| !expired) else {
            self.job = Some(reattached);
            self.finish_job(json!({"exit":model::exit::CANCELLED,"code":"declined","key":null,"params":{},"detail":""}), false);
            return;
        };
        let tid = match &plan {
            model::Reattach::Transaction(tid) => Some(tid.clone()),
            _ => None,
        };
        let listed = tid.as_ref().and_then(|tid| {
            let reply = conn
                .call_sync(Some(SERVICE), PATH, IFACE, "GetTransactionList", None, None, gio::DBusCallFlags::NO_AUTO_START, 3000, gio::Cancellable::NONE)
                .ok()?;
            let list: Vec<String> = (0..reply.child_value(0).n_children())
                .filter_map(|i| reply.child_value(0).child_value(i).str().map(str::to_owned))
                .collect();
            list.contains(tid).then(|| tid.clone())
        });
        let ours = listed.and_then(|tid| {
            let props = conn
                .call_sync(Some(SERVICE), &tid, PROPERTIES, "GetAll", Some(&(TX_IFACE,).to_variant()), None, gio::DBusCallFlags::NO_AUTO_START, 3000, gio::Cancellable::NONE)
                .ok()?;
            let map = props.child_value(0).get::<HashMap<String, glib::Variant>>()?;
            let get = |k: &str| map.get(k).map(|v| unboxed(v.clone())).and_then(|v| v.get::<u32>());
            let uid = unsafe { libc::getuid() };
            ([Some(model::role::UPDATE_PACKAGES), Some(model::role::REPAIR_SYSTEM)].contains(&get("Role")) && get("Uid") == Some(uid))
                .then_some((tid, props.child_value(0)))
        });
        self.hold_app(true);
        match ours {
            Some((tid, props)) => {
                let mut tx = Tx::new(if reattached.role == "repair" { Purpose::Repair } else { Purpose::Update }, reattached.ids.clone());
                tx.properties(&props);
                tx.subs = self.subscribe_tx(&conn, &tid);
                reattached.tid = Some(tid.clone());
                reattached.step = if reattached.role == "repair" { Step::Repair } else { Step::Update };
                self.txs.insert(tid.clone(), tx);
                self.job = Some(reattached);
                self.start_progress_timer();
                self.job_progress(&tid);
            }
            None => {
                // step 4: the daemon or the transaction is gone; ask GetOldTransactions(5) for the outcome
                self.job = Some(reattached);
                self.start_tx(&conn, Purpose::OldTransactions, "GetOldTransactions", (5u32,).to_variant(), tid.into_iter().collect());
            }
        }
    }
    fn on_old_transactions(&mut self, tx: Tx) {
        let ours = tx.ids.first().cloned();
        let found = tx.old.iter().find(|(path, _, role, _)| {
            Some(path) == ours.as_ref() && [model::role::UPDATE_PACKAGES, model::role::REPAIR_SYSTEM].contains(role)
        });
        let outcome = match found {
            Some((_, true, _, _)) => json!({"exit":model::exit::SUCCESS,"code":null,"key":null,"params":{},"detail":""}),
            Some((_, false, _, data)) => json!({"exit":model::exit::FAILED,"code":"failed","key":"update.err.failed","params":{},"detail":data}),
            None => json!({"exit":null,"code":"unknown_outcome","key":"update.err.unknownOutcome","params":{},"detail":""}),
        };
        self.vanished = false;
        self.finish_job(outcome, true);
    }

    // ---------------------------------------------------------------------------------- events

    fn on_manager(&mut self, member: &str, params: &glib::Variant) {
        match member {
            "UpdatesChanged" => {
                self.rescan(false);
                self.persist();
                // apt or unattended-upgrades changed the system (apt's 20packagekit hook): read the list again
                // once the signals settle, without RefreshCache (review m2)
                self.changed_at = Some(Instant::now());
                self.arm_reread();
            }
            "TransactionListChanged" => {
                let Some((list,)) = params.get::<(Vec<String>,)>() else { return };
                self.foreign_from(list);
            }
            _ => {}
        }
    }
    /// Foreign transactions: an install/update/remove that is not ours (design §4.7). NO_AUTO_START reads.
    fn foreign_from(&mut self, list: Vec<String>) {
        let Some(conn) = self.conn.clone() else { return };
        let ours: Vec<&String> = self.txs.keys().collect();
        let our_tid = self.job.as_ref().and_then(|j| j.tid.clone());
        let mut foreign = Vec::new();
        for path in list.iter().filter(|p| !ours.contains(p)).take(20) {
            let role = conn
                .call_sync(Some(SERVICE), path, PROPERTIES, "Get", Some(&(TX_IFACE, "Role").to_variant()), None, gio::DBusCallFlags::NO_AUTO_START, 2000, gio::Cancellable::NONE)
                .ok()
                .map(|v| unboxed(v.child_value(0)))
                .and_then(|v| v.get::<u32>());
            if role.is_some_and(|role| model::foreign(role, path, our_tid.as_deref())) {
                foreign.push(path.clone());
            }
        }
        if foreign != self.foreign {
            self.foreign = foreign;
            self.persist();
        }
    }
    fn on_owner(&mut self, present: bool) {
        if !present {
            self.presence = self.conn.as_ref().map(presence).unwrap_or(Presence::Absent);
            let active: Vec<String> = self.txs.keys().cloned().collect();
            for path in active {
                self.end_tx(&path);
            }
            self.foreign.clear();
            self.persist();
        } else {
            self.presence = Presence::Running;
            if self.vanished && self.job.is_none() {
                self.vanished = false;
            }
            self.persist();
        }
    }
    /// NM changed one of the two properties: read both again (NO_AUTO_START), publish when they differ.
    fn on_net(&mut self) {
        let next = self.conn.as_ref().map(net::read).unwrap_or_default();
        if next != self.net {
            self.net = next;
            self.persist();
        }
    }
    fn on_watchdog(&mut self) {
        let Some(conn) = self.conn.clone() else { return };
        let silent: Vec<String> = self.txs.iter().filter(|(_, tx)| tx.last_signal.elapsed() >= SILENT).map(|(p, _)| p.clone()).collect();
        for path in silent {
            let target = path.clone();
            conn.call(Some(SERVICE), &path, PROPERTIES, "GetAll", Some(&(TX_IFACE,).to_variant()), None, gio::DBusCallFlags::NO_AUTO_START, 5000, gio::Cancellable::NONE, move |reply| {
                with(move |e| match reply {
                    Ok(props) => e.on_tx_properties(&target, &props.child_value(0)),
                    Err(error) if uncertain(&error) => e.end_tx(&target),
                    Err(_) => {}
                });
            });
        }
        if self.job.as_ref().is_some_and(|j| j.written.elapsed() >= JOB_REWRITE) {
            self.write_job();
        }
    }
    /// (Re)start the debounce timer of the re-read after `UpdatesChanged`.
    fn arm_reread(&mut self) {
        let source = glib::timeout_source_new(REREAD_DELAY, Some("update-reread"), glib::Priority::DEFAULT, || {
            with(Engine::reread);
            glib::ControlFlow::Break
        });
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        if let Some(old) = self.reread_timer.replace(source) {
            old.destroy();
        }
    }
    /// The re-read after `UpdatesChanged` (review m2): GetUpdates without RefreshCache, the classifier and the
    /// sizes, exactly as the re-check after a job (`checking`, so an install waits for it, F16). Packages that
    /// apt or unattended-upgrades installed meanwhile leave the rows and `system.updates`. Not needed when a
    /// GetUpdates started after the signal (a check, or the re-check after our job), and never before the first
    /// check (that one is the user's, design §4.10). While a check or our job runs: again once it ended.
    fn reread(&mut self) {
        if let Some(source) = self.reread_timer.take() {
            source.destroy();
        }
        let Some(at) = self.changed_at else { return };
        if self.list_started.is_some_and(|started| started >= at) || self.checked_at.is_none() {
            self.changed_at = None;
            return;
        }
        if self.checking || self.job.is_some() {
            self.arm_reread();
            return;
        }
        let Ok(conn) = self.ready(lang_of(&self.shared)) else {
            self.changed_at = None;
            return;
        };
        self.changed_at = None;
        self.checking = true;
        self.start_tx(&conn, Purpose::GetUpdates, "GetUpdates", (model::FILTER_NONE,).to_variant(), Vec::new());
        self.persist();
    }
    /// The restart scan (design §4.8): at most once a minute unless `force`.
    fn rescan(&mut self, force: bool) {
        if !force && self.scanned.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        let paths = &self.shared.setup.paths;
        let own_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.to_path_buf())).unwrap_or_default();
        self.scan = restart::scan(paths, unsafe { libc::getuid() }, &own_dir);
        self.scanned = Some(Instant::now());
    }

    // WU-FIX (GX1 review n5): a receiver that deferred its restart runs it after all (`shell.restartResumed`,
    // update/restart.rs): scan again after its exec, so the Win11 restart InfoBar goes away with the last old process.
    /// Subscribe on the pk thread's context: the event's handler runs on the events thread and only invokes the
    /// rescan schedule here (no wait, no lock across threads).
    fn watch_restarts(&self) {
        let context = glib::MainContext::ref_thread_default();
        let me = std::process::id();
        self.shared.ctx.events.on(
            restart::RESUMED_EVENT,
            Arc::new(move |data: &Value| {
                if restart::resumed_elsewhere(data, me) {
                    context.invoke(|| {
                        with(Engine::rescan_after_restart);
                    });
                }
            }),
        );
    }
    /// Start (again) the rescans of [`restart::RESCAN_AFTER_RESUMED`].
    fn rescan_after_restart(&mut self) {
        if let Some(source) = self.restart_rescan.take() {
            source.destroy();
        }
        self.arm_restart_rescan(0);
    }
    fn arm_restart_rescan(&mut self, step: usize) {
        let Some(delay) = restart::RESCAN_AFTER_RESUMED.get(step).copied() else { return };
        let source = glib::timeout_source_new(delay, Some("update-restart-rescan"), glib::Priority::DEFAULT, move || {
            with(move |e| e.restart_rescan_step(step));
            glib::ControlFlow::Break
        });
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.restart_rescan = Some(source);
    }
    /// One rescan: publish it when it changed; scan again later while a replaced component of ours is still mapped.
    fn restart_rescan_step(&mut self, step: usize) {
        self.restart_rescan = None;
        let before = self.scan.clone();
        self.rescan(true);
        if self.scan != before {
            self.persist();
        }
        if !self.scan.shell_parts.is_empty() {
            self.arm_restart_rescan(step + 1);
        }
    }
    // WU-FIX end
}
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(conn) = &self.conn {
            for sub in self.subs.drain(..) {
                conn.signal_unsubscribe(sub);
            }
            for (_, mut tx) in self.txs.drain() {
                for sub in tx.subs.drain(..) {
                    conn.signal_unsubscribe(sub);
                }
            }
            // A private test bus connection is ours alone: close it now, so the fake sees this caller vanish
            // before a mirror takes over (as when the process exits). The real system bus is the process-wide
            // shared connection other services use too; it is left to the process exit.
            if matches!(self.shared.setup.bus, BusTarget::Address(_)) {
                let _ = conn.close_sync(gio::Cancellable::NONE);
            }
        }
        for source in self.sources.drain(..) {
            source.destroy();
        }
        if let Some(source) = self.progress_source.take() {
            source.destroy();
        }
        if let Some(source) = self.hold_timer.take() {
            source.destroy();
        }
        if let Some(source) = self.reread_timer.take() {
            source.destroy();
        }
        // WU-FIX (GX1 review n5)
        if let Some(source) = self.restart_rescan.take() {
            source.destroy();
        }
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).take(MAX_PACKAGES).collect()).unwrap_or_default()
}
/// One `UpdateDetail` record → the API's detail object (only https: links, design §3.3).
fn update_detail(v: &glib::Variant) -> Option<Value> {
    if v.n_children() < 12 {
        return None;
    }
    let s = |i: usize| v.child_value(i).get::<String>().unwrap_or_default();
    let list = |i: usize| -> Vec<String> {
        v.child_value(i)
            .get::<Vec<String>>()
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.starts_with("https://") && u.len() <= 2048)
            .take(100)
            .collect()
    };
    let id = s(0);
    if id.is_empty() {
        return None;
    }
    Some(json!({"id":id,"updateText":s(7).chars().take(20_000).collect::<String>(),
                "changelog":s(8).chars().take(50_000).collect::<String>(),"cves":list(5),"bugs":list(4),
                "restart":v.child_value(6).get::<u32>().unwrap_or(0),"issued":s(10),"updated":s(11)}))
}

/// The engine's job ending without a bus (the bus paths are GW2b's): what the snapshot says after `Finished`,
/// the keep-alive after a job and during a confirm, and a job that was paused for a confirm (review F2, F3, F7,
/// F14). The context is dry, so `notify()` never opens a session bus here.
#[cfg(test)]
mod engine_tests {
    use super::super::tests::{Fixture, pkg, write};
    use super::*;

    fn engine(f: &Fixture) -> Engine {
        let ctx = f.ctx("settings", true, "en");
        let setup = super::super::paths::resolve(&f.env_no_bus()).unwrap();
        let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        Engine::new(Arc::new(Shared { ctx, setup, store, stats: Default::default(), rev: Default::default() }))
    }
    fn job(ids: Vec<String>, row_info: Vec<Value>, step: Step) -> Job {
        Job {
            id: "0123456789abcdef0123456789abcdef".into(),
            tid: Some("/1_x".into()),
            role: "update",
            ids,
            rows: row_info.iter().filter_map(|r| r["id"].as_str().map(str::to_owned)).collect(),
            row_info,
            started_at: 1,
            step,
            installs: Vec::new(),
            removals: Vec::new(),
            download_size: None,
            confirm: None,
            committing: false,
            status: 0,
            pct: None,
            speed: 0,
            remaining: 0,
            download_left: None,
            allow_cancel: false,
            reattached: false,
            states: HashMap::new(),
            restart: false,
            written: Instant::now(),
            _lock: None,
        }
    }
    fn held(e: &Engine) -> bool {
        e.shared.stats.held.load(Ordering::SeqCst)
    }

    #[test]
    fn a_finished_job_leaves_no_installed_package_in_the_snapshot() {
        let f = Fixture::new("finish");
        let mut e = engine(&f);
        e.pkgs = vec![pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", 8, "ubuntu-resolute-security"), pkg("zlib1g", "1:1.3-1", 5, "u")];
        e.checked_at = Some(1);
        e.presence = Presence::Running;
        e.repair_required = true;
        let rows = model::group(&e.pkgs, "26.04.1 LTS");
        let ids = model::install_ids(&rows, Some(&["soup".into()])).unwrap();
        let info = rows.iter().filter(|r| r.id == "soup").map(model::row_info).collect();
        e.job = Some(job(ids, info, Step::Update));
        e.hold_app(true);
        e.finish_job(json!({"exit":model::exit::SUCCESS,"code":null,"key":null,"params":{},"detail":""}), true);
        // F2: the installed package is gone at once, nothing says "checking" (no bus here, so no re-check)
        let snapshot = e.shared.store.read(SNAPSHOT).unwrap();
        let names: Vec<&str> = snapshot["packages"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
        assert_eq!(names, ["zlib1g"]);
        assert_eq!(snapshot["checking"], false);
        assert_eq!(snapshot["totals"]["count"], 1);
        assert!(!held(&e), "no re-check: the keep-alive ends at once");
        // F14: a success clears repairRequired
        assert_eq!(snapshot["repairRequired"], false);
        // F3: the job's row is installed, the rest is still offered
        assert_eq!(snapshot["lastJob"]["rows"][0]["id"], "soup");
        let job_file = e.shared.store.read(JOB).unwrap();
        assert!(job_file["finishedAt"].is_u64());
        let view = super::super::compose(&e.shared, Some(&snapshot), Some(&job_file), None, false);
        let states: Vec<(&str, &str)> = view["rows"].as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap(), r["state"].as_str().unwrap())).collect();
        assert_eq!(states, [("soup", "installed"), ("other", "pendingDownload")]);
        assert_eq!(view["phase"], "available");
        assert_eq!(e.shared.store.jobs().len(), 1);
        // the next check ends the record
        e.last_job = None;
        e.persist();
        assert!(e.shared.store.read(SNAPSHOT).unwrap()["lastJob"].is_null());
    }

    #[test]
    fn a_failed_job_keeps_its_packages_and_marks_its_rows_failed() {
        let f = Fixture::new("failed");
        let mut e = engine(&f);
        e.pkgs = vec![pkg("zlib1g", "1:1.3-1", 5, "u")];
        e.checked_at = Some(1);
        e.presence = Presence::Running;
        let rows = model::group(&e.pkgs, "x");
        let info = rows.iter().map(model::row_info).collect();
        e.job = Some(job(rows[0].ids.clone(), info, Step::Update));
        e.finish_job(json!({"exit":2,"code":"dpkg","key":"update.err.dpkg","params":{},"detail":"E: x"}), true);
        let snapshot = e.shared.store.read(SNAPSHOT).unwrap();
        assert_eq!(snapshot["packages"].as_array().unwrap().len(), 1, "unknown which got installed: the re-check decides");
        let view = super::super::compose(&e.shared, Some(&snapshot), None, None, false);
        assert_eq!(view["phase"], "error");
        assert_eq!(view["rows"][0]["state"], "failed");
        assert_eq!(view["rows"][0]["error"]["key"], "update.err.dpkg");
    }

    #[test]
    fn the_keep_alive_lasts_until_the_recheck_and_not_through_a_confirm() {
        let f = Fixture::new("hold");
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let mut e = engine(&f);
                // after a job: held until the re-check answered
                e.hold_app(true);
                e.checking = true;
                e.hold_until_checked();
                assert!(held(&e) && e.hold_timer.is_some());
                e.checked();
                assert!(!held(&e) && e.hold_timer.is_none() && !e.checking);
                // a new job took the hold meanwhile: the re-check does not release it
                e.hold_app(true);
                e.hold_until_checked();
                e.job = Some(job(Vec::new(), Vec::new(), Step::Simulate));
                e.checked();
                assert!(held(&e), "the running job keeps its hold");
                // F7: a confirm pause releases the hold (no transaction runs while the user decides)
                e.pkgs = vec![pkg("zlib1g", "1:1.3-1", 5, "u")];
                let mut tx = Tx::new(Purpose::Simulate, Vec::new());
                tx.packages.push((13, "libold1;1;amd64;installed".into(), String::new()));
                tx.finished = Some(model::exit::SUCCESS);
                e.on_simulated(tx, model::exit::SUCCESS);
                assert_eq!(e.job.as_ref().map(|j| j.step), Some(Step::Confirm));
                assert_eq!(e.job.as_ref().unwrap().confirm.as_ref().unwrap()["kind"], "removals");
                assert!(!held(&e), "no hold during the confirm");
            })
            .unwrap();
    }

    /// Review M1/m1: what the hero shows of a running job reaches the page as `update.changed` (the page re-reads
    /// the snapshot only then): waiting for the lock, unattended-upgrades holding it, waiting for authentication,
    /// downloading (with the total the download filled in), installing. Progress alone stays `update.progress`.
    #[test]
    fn a_new_stage_of_the_running_job_is_published() {
        let f = Fixture::new("stage");
        let mut e = engine(&f);
        let changed: Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
        {
            let changed = changed.clone();
            e.shared.ctx.events.on("update.changed", Arc::new(move |d: &Value| changed.lock().unwrap().push(d.clone())));
        }
        e.pkgs = vec![pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", 8, "ubuntu-resolute-security")];
        e.checked_at = Some(1);
        e.presence = Presence::Running;
        let rows = model::group(&e.pkgs, "x");
        let info = rows.iter().map(model::row_info).collect();
        e.job = Some(job(rows[0].ids.clone(), info, Step::Update));
        e.txs.insert("/1_x".into(), Tx::new(Purpose::Update, Vec::new()));
        e.persist();
        let step = |e: &mut Engine, status: u32, left: Option<u64>| -> bool {
            let before = e.rev;
            let tx = e.txs.get_mut("/1_x").unwrap();
            tx.status = status;
            tx.download_left = left;
            tx.pct = tx.pct.map(|p| p + 1).or(Some(0));
            e.job_progress("/1_x");
            e.rev != before
        };
        assert!(step(&mut e, 30, None), "waiting for the lock");
        assert!(!step(&mut e, 30, None), "the same wait");
        write(&f.root.join("proc/4242/comm"), "unattended-upgr\n");
        e.txs.get_mut("/1_x").unwrap().holder_checked = None;
        assert!(step(&mut e, 30, None), "unattended-upgrades holds the lock");
        assert!(step(&mut e, 31, None), "waiting for authentication");
        assert!(step(&mut e, 8, Some(5000)), "downloading, the download's total");
        assert!(!step(&mut e, 8, Some(4000)), "download progress only");
        assert!(step(&mut e, 16, None), "installing");
        assert!(!step(&mut e, 10, None), "still installing");
        let seen = changed.lock().unwrap().clone();
        let got: Vec<(Value, Value, Value, Value)> = seen[1..]
            .iter()
            .map(|d| (d["phase"].clone(), d["job"]["status"].clone(), d["job"]["lockHolder"].clone(), d["totals"]["downloadSize"].clone()))
            .collect();
        let n = Value::Null;
        assert_eq!(
            got,
            [
                (json!("waiting"), json!(30), n.clone(), n.clone()),
                (json!("waiting"), json!(30), json!("unattended"), n.clone()),
                (json!("waiting"), json!(31), n.clone(), n.clone()),
                (json!("downloading"), json!(8), n.clone(), json!(5000)),
                (json!("installing"), json!(16), n.clone(), json!(5000)),
            ]
        );
        assert_eq!(e.shared.store.read(SNAPSHOT).unwrap()["downloadSize"], 5000, "the filled total is persisted");
        // a simulate that reports 0 leaves the total unknown (the download fills it in)
        let mut tx = Tx::new(Purpose::Simulate, Vec::new());
        tx.download_left = Some(0);
        e.job.as_mut().unwrap().step = Step::Simulate;
        e.job.as_mut().unwrap().download_size = None;
        e.download_size = None;
        e.on_simulated(tx, model::exit::SUCCESS);
        assert_eq!(e.job.as_ref().unwrap().download_size, None);
        assert!(e.shared.store.read(SNAPSHOT).unwrap()["downloadSize"].is_null());
    }

    /// Review m2: the re-read after `UpdatesChanged` never makes the first list, waits for a running check or job,
    /// and is dropped when a GetUpdates started after the signal (the bus test `updates-changed` runs the read).
    #[test]
    fn the_reread_after_updates_changed_waits_and_is_not_repeated() {
        let f = Fixture::new("reread");
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let mut e = engine(&f);
                e.presence = Presence::Running;
                let signal = |e: &mut Engine| e.on_manager("UpdatesChanged", &().to_variant());
                signal(&mut e);
                assert!(e.reread_timer.is_some() && e.changed_at.is_some(), "debounced");
                e.reread();
                assert!(e.changed_at.is_none() && e.reread_timer.is_none(), "never checked: no first list from a signal");
                e.checked_at = Some(1);
                e.checking = true;
                signal(&mut e);
                e.reread();
                assert!(e.changed_at.is_some() && e.reread_timer.is_some(), "waits for the running check");
                e.list_started = Some(Instant::now());
                e.checking = false;
                e.reread();
                assert!(e.changed_at.is_none() && e.reread_timer.is_none(), "that check's GetUpdates came after the signal");
                e.job = Some(job(Vec::new(), Vec::new(), Step::Confirm));
                signal(&mut e);
                e.reread();
                assert!(e.changed_at.is_some() && e.reread_timer.is_some(), "waits for our job");
                // the job ended without a re-check (declined): read now (no bus here, so nothing starts)
                e.job = None;
                e.reread();
                assert!(e.changed_at.is_none() && !e.checking);
            })
            .unwrap();
    }

    // WU-FIX (GX1 review n5): after a deferred receiver's shell.restartResumed the owner scans again until that
    // process runs the new build, then publishes the restart state without the Win11 InfoBar's components.
    #[test]
    fn a_deferred_receiver_that_restarts_clears_the_restart_infobar() {
        let f = Fixture::new("resumed");
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let mut e = engine(&f);
                let changed: Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
                {
                    let changed = changed.clone();
                    e.shared.ctx.events.on("update.changed", Arc::new(move |d: &Value| changed.lock().unwrap().push(d.clone())));
                }
                // a native Win11 Explorer of this user that still runs its replaced binary and maps a replaced GTK 4
                let own = std::env::current_exe().unwrap().parent().unwrap().join("win11-gtk-explorer");
                let explorer = f.root.join("proc/4300");
                let uid = unsafe { libc::getuid() };
                write(&explorer.join("status"), &format!("Name:\twin11-gtk-explor\nPPid:\t1\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"));
                write(&explorer.join("comm"), "win11-gtk-explor\n");
                let gtk4 = "7f10-7f11 r-xp 00000000 08:02 31 /usr/lib/x86_64-linux-gnu/libgtk-4.so.1.2200.4 (deleted)\n";
                write(&explorer.join("maps"), gtk4);
                std::os::unix::fs::symlink(format!("{} (deleted)", own.display()), explorer.join("exe")).unwrap();
                e.rescan(true);
                assert_eq!(e.scan.shell_parts, ["gtk4", "self"], "the InfoBar of the restarted Settings");
                e.persist();
                let rev = e.rev;
                // the event arms the first rescan; its exec has not happened yet: nothing new is published, and
                // the next rescan is armed
                e.rescan_after_restart();
                assert!(e.restart_rescan.is_some());
                e.restart_rescan_step(0);
                assert_eq!((e.rev, e.scan.shell_parts.len()), (rev, 2), "unchanged: not published again");
                assert!(e.restart_rescan.is_some(), "scans again later");
                // the exec: the new image, no replaced mapping
                std::fs::remove_file(explorer.join("exe")).unwrap();
                std::os::unix::fs::symlink(&own, explorer.join("exe")).unwrap();
                write(&explorer.join("maps"), "7f10-7f11 r-xp 00000000 08:02 32 /usr/lib/x86_64-linux-gnu/libgtk-4.so.1.2200.5\n");
                e.restart_rescan_step(1);
                assert!(e.scan.shell_parts.is_empty());
                assert!(e.rev > rev, "published");
                let last = changed.lock().unwrap().last().cloned().unwrap();
                assert_eq!((last["restart"]["shell"].clone(), last["restart"]["shellParts"].clone()), (json!(false), json!([])));
                assert!(e.restart_rescan.is_none(), "nothing left to clear: no further rescan");
                // the schedule ends after its last step even while something stays replaced
                e.arm_restart_rescan(restart::RESCAN_AFTER_RESUMED.len());
                assert!(e.restart_rescan.is_none());
            })
            .unwrap();
        // only another process's event counts (the owner's own exec ends its engine anyway)
        let me = std::process::id();
        assert!(restart::resumed_elsewhere(&json!({"pid":me + 1,"kind":"explorer","wait":"files"}), me));
        assert!(!restart::resumed_elsewhere(&json!({"pid":me}), me));
        assert!(!restart::resumed_elsewhere(&json!({"kind":"explorer"}), me));
    }
    // WU-FIX end

    #[test]
    fn a_job_paused_for_a_confirm_ends_as_declined_in_the_next_owner() {
        let f = Fixture::new("expired");
        let mut e = engine(&f);
        let boot = store::boot_id(&e.shared.setup.paths).unwrap();
        e.shared
            .store
            .write(JOB, &json!({"version":1,"jobId":"0123456789abcdef0123456789abcdef","tid":"/7_x","bootId":boot,"role":"update",
                                "ids":["zlib1g;1:1.3-1;amd64;u"],"rows":["other"],"confirm":{"kind":"metered","size":5},"committing":false}))
            .unwrap();
        e.reattach();
        assert!(e.job.is_none());
        let file = e.shared.store.read(JOB).unwrap();
        assert_eq!(file["outcome"]["code"], "declined", "the simulate's success is no install");
        assert!(file["finishedAt"].is_u64());
        assert!(!held(&e));
    }
}
