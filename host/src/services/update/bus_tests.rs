//! Bus tests of the update service against GW1's fake PackageKit and fake NetworkManager (design §9.2; GW2b).
//!
//! `#[ignore]`: they run **only** inside GW1's bubblewrap layer, each test in its own `run.sh exec`:
//! `tools/heavy.sh --tag gw2b-bus -- tests/update/run.sh bus-tests -- <the lib test binary>` (the expectations per
//! test are in `tests/update/stubs/bus-tests.json`). The layer has a private `/run` (no real system bus, no live
//! session bus), `--unshare-net`, stubs first on PATH, the seams of contract C10, strace on connect, sendto and
//! sendmsg, and after the run the stub-log, `CallCount()` and `State().violations` checks. Never `cargo test
//! --include-ignored`, never outside that layer: [`fake_bus`] refuses.
//!
//! A scenario resets the fixture root, `Load()`s and `Run()`s a timeline, starts the services with this process's
//! own seams (`Update::new`), drives them through the API as the page does, and then compares the fake's
//! `CallsJson()` (PackageKit calls in order, NetworkManager reads and Notifications by count), the stub log since
//! the scenario began and `State().violations` with what the scenario expects ([`verify`]). Texts are compared
//! with `host/i18n/update.tsv` through `crate::i18n`, in the scenario's language.
use super::{
    Mode, Update, guard,
    paths::{self, BusTarget, MARKER, Setup},
    store,
};
use crate::{
    events::Events,
    files::{ApiError, ApiResult},
    i18n::{self, Lang},
    services::ServiceContext,
};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak, atomic::Ordering},
    time::{Duration, Instant},
};

/// The variable in which `tests/update/run.sh` records `readlink /proc/self/ns/net` **outside** the bwrap layer
/// (not a `WIN11_UPDATE_*` name: an unknown seam variable fails the seam check closed).
pub(super) const HOST_NETNS: &str = "WIN11_TEST_HOST_NETNS";

/// The run's private bus and seams, or a panic (fail closed). Inside the bwrap layer of design §4.11 only:
/// * the real system-bus socket is invisible (`--tmpfs /run`);
/// * the live session bus is invisible (`/run/user` empty) and `DBUS_SESSION_BUS_ADDRESS` is not under
///   `/run/user/` (the service uses the session bus for `notify()`; review F10);
/// * the network namespace is not the host's (`--unshare-net`: no downloads, no abstract socket of the live
///   session), compared with [`HOST_NETNS`];
/// * every `WIN11_UPDATE_*` seam passes (contract C10 for the bus), and the bus is a test address.
fn fake_bus() -> Setup {
    // anything but NotFound counts as visible (review n4)
    assert!(
        !["/run/dbus/system_bus_socket", "/var/run/dbus/system_bus_socket"].iter().any(|p| paths::maybe_present(Path::new(p))),
        "the real system bus is visible: bus tests run only inside the bwrap layer (tests/update/run.sh)"
    );
    let uid = unsafe { libc::getuid() };
    assert!(!paths::maybe_present(Path::new(&format!("/run/user/{uid}/bus"))), "the live session bus is visible: /run is not private");
    let session = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    assert!(!session.contains("/run/user/"), "DBUS_SESSION_BUS_ADDRESS is the live session bus: {session}");
    let host = std::env::var(HOST_NETNS).unwrap_or_default();
    let here = fs::read_link("/proc/self/ns/net").map(|p| p.display().to_string()).unwrap_or_default();
    assert!(host.starts_with("net:["), "{HOST_NETNS} is not set by the runner");
    assert!(!here.is_empty() && here != host, "the host network namespace: --unshare-net is missing");
    let setup = paths::process_setup().expect("the WIN11_UPDATE_* seams of tests/update/run.sh (contract C10)");
    assert!(setup.test, "no test seams set");
    assert!(matches!(setup.bus, BusTarget::Address(_)), "no WIN11_UPDATE_PK_BUS");
    setup
}

// ------------------------------------------------------------------------------------------------ the fake

const CONTROL: &str = "io.github.win11shell.FakePackageKit";
const CONTROL_PATH: &str = "/Control";
const PK: &str = "org.freedesktop.PackageKit";
const PK_PATH: &str = "/org/freedesktop/PackageKit";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";
const BOOT_ID: &str = "6a2e9f40-1b3c-4d5e-8f70-0123456789ab";
const OS_RELEASE: &str = "NAME=\"Ubuntu\"\nVERSION=\"26.04.1 LTS (Resolute Raccoon)\"\nVERSION_ID=\"26.04\"\nVERSION_CODENAME=resolute\n";
/// What `/proc/<pid>/cmdline` of Ubuntu's unpacked release upgrader looks like (design §6.4 step 5).
const UPGRADER: &str = "python3\0-s\0/tmp/ubuntu-release-upgrader-x1y2z3/stonking\0--mode=server\0--frontend=DistUpgradeViewText\0";

/// The test's own connection to the fake's bus: the control interface, and PackageKit calls that the timelines
/// expect from a second client (apt's hook, another canceller).
struct Fake {
    conn: gio::DBusConnection,
    run: PathBuf,
    fixture: PathBuf,
}
impl Fake {
    fn new() -> Self {
        let setup = fake_bus();
        let BusTarget::Address(address) = &setup.bus else { panic!("no test bus") };
        let conn = gio::DBusConnection::for_address_sync(
            address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .expect("the fake's private bus");
        let run = PathBuf::from(std::env::var_os("WIN11_FAKE_RUN_DIR").expect("WIN11_FAKE_RUN_DIR"));
        let fixture = PathBuf::from(std::env::var_os(paths::VAR_FIXTURE_ROOT).expect("the fixture root"));
        Self { conn, run, fixture }
    }
    fn control(&self, method: &str, args: Option<glib::Variant>) -> glib::Variant {
        self.conn
            .call_sync(Some(CONTROL), CONTROL_PATH, CONTROL, method, args.as_ref(), None, gio::DBusCallFlags::NONE, 15_000, gio::Cancellable::NONE)
            .unwrap_or_else(|e| panic!("fake {method}: {e}"))
    }
    /// The fixture root back to its base state, then `Load(timeline)` (which writes the timeline's own fixtures).
    fn load(&self, timeline: &str) {
        self.reset_fixture();
        self.control("Load", Some((timeline,).to_variant()));
    }
    fn run(&self) {
        self.control("Run", None);
    }
    fn step(&self) -> String {
        self.control("Step", None).child_value(0).str().unwrap_or("").to_owned()
    }
    fn set_daemon(&self, present: bool) {
        self.control("SetDaemon", Some((present,).to_variant()));
    }
    fn json(&self, method: &str) -> Value {
        let text = self.control(method, None).child_value(0).str().unwrap_or("null").to_owned();
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
    fn calls(&self) -> Vec<Value> {
        self.json("CallsJson").as_array().cloned().unwrap_or_default()
    }
    fn state(&self) -> Value {
        self.json("State")
    }
    fn call_count(&self) -> usize {
        self.control("CallCount", None).child_value(0).get::<u32>().unwrap_or(u32::MAX) as usize
    }
    /// Every stub call of this run (`<run>/stub-calls.jsonl`, written by `tests/update/stubs/stub.py`).
    fn stub_log(&self) -> Vec<Value> {
        fs::read_to_string(self.run.join("stub-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
    /// A PackageKit call by the test itself (a second client of the timeline).
    fn pk_call(&self, path: &str, iface: &str, method: &str, args: Option<glib::Variant>) -> Result<glib::Variant, glib::Error> {
        self.conn
            .call_sync(Some(PK), path, iface, method, args.as_ref(), None, gio::DBusCallFlags::NONE, 10_000, gio::Cancellable::NONE)
    }
    /// The unique names on the fake's bus (a service that connects adds one).
    fn names(&self) -> Vec<String> {
        let reply = self
            .conn
            .call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", None, None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE)
            .expect("ListNames");
        let mut names: Vec<String> = reply.child_value(0).get::<Vec<String>>().unwrap_or_default().into_iter().filter(|n| n.starts_with(':')).collect();
        names.sort();
        names
    }
    /// Base fixture: this test's boot id and os-release, the directories the file watch needs; nothing else.
    fn reset_fixture(&self) {
        for entry in fs::read_dir(&self.fixture).expect("the fixture root").flatten() {
            if entry.file_name() == MARKER {
                continue;
            }
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                fs::remove_dir_all(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
        }
        write(&self.fixture.join("proc/sys/kernel/random/boot_id"), &format!("{BOOT_ID}\n"));
        write(&self.fixture.join("etc/os-release"), OS_RELEASE);
        fs::create_dir_all(self.fixture.join("run")).unwrap();
        fs::create_dir_all(self.fixture.join("var/log/apt")).unwrap();
    }
    /// The transaction streams of the fake that wait at `gate` (e.g. `vanish`).
    fn armed(&self, gate: &str) -> bool {
        self.state()["streams"].as_array().into_iter().flatten().any(|s| s["armed"] == gate)
    }
    fn transactions(&self) -> Vec<Value> {
        self.state()["transactions"].as_array().cloned().unwrap_or_default()
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

// ------------------------------------------------------------------------------------------------ hosts

/// The directory of one scenario: a shared cache (the update state files), and per host a config and events dir.
/// Short, because the event sockets live below it (`sockaddr_un` holds 108 bytes).
struct Scenario {
    dir: PathBuf,
}
impl Scenario {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("b{}", crate::events::nonce()));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
    fn cache(&self) -> PathBuf {
        self.dir.join("cache")
    }
    fn store(&self) -> store::Store {
        store::Store::open(self.cache().join("update")).unwrap()
    }
}
impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// What a host saw on `update.changed`: the event's `rev`, the whole event (what the page gets: phase, job,
/// totals, …), the snapshot file at that moment and the keep-alive.
#[derive(Clone, Debug)]
struct Seen {
    rev: Option<u64>,
    event: Value,
    snapshot: Value,
    held: bool,
}
type Log<T> = Arc<Mutex<Vec<T>>>;

/// One Win11 host of `kind` with its update service. Its observers record `update.changed` (with the snapshot
/// file as it is then: for its own events that is inside `persist()`, on the pk thread), `update.progress` and
/// `update.reply`.
struct Host {
    ctx: Arc<ServiceContext>,
    service: Arc<Update>,
    seen: Log<Seen>,
    progress: Log<(Instant, Value)>,
    replies: Log<Value>,
}
impl Host {
    fn new(scn: &Scenario, label: &str, kind: &str, lang: Lang, dry: bool, events: Option<&Path>) -> Self {
        let config = scn.dir.join(format!("{label}-config"));
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("settings.json"), json!({"language":lang.code()}).to_string()).unwrap();
        let events_dir = events.map(Path::to_path_buf).unwrap_or_else(|| scn.dir.join(format!("{label}-events")));
        let events = Events::new(&events_dir).unwrap();
        let ctx = ServiceContext::new(events, config, scn.cache(), scn.dir.clone(), kind.into(), dry).unwrap();
        let (seen, progress, replies): (Log<Seen>, Log<(Instant, Value)>, Log<Value>) = Default::default();
        let slot: Arc<Mutex<Weak<Update>>> = Arc::new(Mutex::new(Weak::new()));
        let cache = scn.cache().join("update");
        {
            let (seen, slot) = (seen.clone(), slot.clone());
            ctx.events.on(
                "update.changed",
                Arc::new(move |data: &Value| {
                    let snapshot = store::Store::open(cache.clone()).ok().and_then(|s| s.read(store::SNAPSHOT)).unwrap_or(Value::Null);
                    let held = slot
                        .lock()
                        .unwrap()
                        .upgrade()
                        .and_then(|u| u.shared.clone())
                        .is_some_and(|s| s.stats.held.load(Ordering::SeqCst));
                    seen.lock().unwrap().push(Seen { rev: data["rev"].as_u64(), event: data.clone(), snapshot, held });
                }),
            );
        }
        {
            let progress = progress.clone();
            ctx.events.on("update.progress", Arc::new(move |data: &Value| progress.lock().unwrap().push((Instant::now(), data.clone()))));
        }
        {
            let replies = replies.clone();
            ctx.events.on("update.reply", Arc::new(move |data: &Value| replies.lock().unwrap().push(data.clone())));
        }
        let service = Update::new(ctx.clone()).unwrap();
        *slot.lock().unwrap() = Arc::downgrade(&service);
        Self { ctx, service, seen, progress, replies }
    }
    /// A `settings` host that won the owner lock (its first status waits for the pk thread's init).
    fn owner(scn: &Scenario, label: &str, lang: Lang) -> Self {
        let host = Self::new(scn, label, "settings", lang, false, None);
        assert_eq!(host.service.mode(), Mode::Owner, "{label} is the owner");
        host.status();
        host
    }
    fn call(&self, method: &str, args: Value) -> ApiResult {
        self.service.call(method, &args).unwrap_or_else(|| panic!("{method} is an update method"))
    }
    fn status(&self) -> Value {
        self.call("update.status", json!({})).expect("update.status")
    }
    fn held(&self) -> bool {
        self.service.shared.as_ref().is_some_and(|s| s.stats.held.load(Ordering::SeqCst))
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    /// `shutdown()`, timed (design §9.2: < 100 ms while a transaction runs).
    fn shutdown(&self) -> Duration {
        let started = Instant::now();
        self.service.shutdown();
        started.elapsed()
    }
    fn wait(&self, what: &str, timeout: Duration, done: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            let status = self.status();
            if done(&status) {
                return status;
            }
            assert!(started.elapsed() < timeout, "timed out after {timeout:?} waiting for {what}; last status: {}", short(&status));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// `update.check` and its end (a number in `checkedAt`, nothing refreshing).
    fn check(&self) -> Value {
        assert_eq!(self.call("update.check", json!({})).expect("update.check")["started"], true);
        self.checked()
    }
    fn checked(&self) -> Value {
        self.wait("the check", LONG, |s| s["checkedAt"].is_u64() && s["refreshing"] == false && s["job"].is_null())
    }
    fn install(&self, args: Value) -> String {
        let r = self.call("update.install", args).expect("update.install");
        r["jobId"].as_str().filter(|id| store::is_token(id)).expect("a job id").to_owned()
    }
    /// No job any more and the re-check after it answered.
    fn done(&self, what: &str) -> Value {
        self.wait(what, LONG, |s| s["job"].is_null() && s["refreshing"] == false)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.service.shutdown();
        self.ctx.events.shutdown();
    }
}

const LONG: Duration = Duration::from_secs(30);
const SHUTDOWN_LIMIT: Duration = Duration::from_millis(100);

fn short(v: &Value) -> String {
    let text = v.to_string();
    text.chars().take(1500).collect()
}
fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < timeout, "timed out after {timeout:?} waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
/// A catalogue text in `lang`, as the service renders it (`{product}` filled).
fn text(lang: Lang, key: &str) -> String {
    i18n::trf(lang, key, &json!({"product": i18n::tr(lang, "update.product")}))
}
fn assert_error(e: &ApiError, code: &str, key: &str, lang: Lang, what: &str) {
    assert_eq!((e.code.as_str(), e.key.as_deref()), (code, Some(key)), "{what}: {e:?}");
    assert_eq!(e.message, text(lang, key), "{what}: the {} text of {key}", lang.code());
}
/// The snapshot's error and its text in `lang`.
fn assert_snapshot_error(s: &Value, key: &str, lang: Lang, what: &str) {
    assert_eq!(s["error"]["key"], key, "{what}: {}", short(&s["error"]));
    assert_eq!(s["error"]["message"], text(lang, key), "{what}");
}
/// (row id, row state) in the page's order.
fn rows(s: &Value) -> Vec<(String, String)> {
    s["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (r["id"].as_str().unwrap_or("").to_owned(), r["state"].as_str().unwrap_or("").to_owned()))
        .collect()
}
fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
}
/// The `update.changed` events a host got while our job ran (`job` an object), in order.
fn job_events(seen: &[Seen]) -> Vec<Value> {
    seen.iter().filter(|v| v.event["job"].is_object()).map(|v| v.event.clone()).collect()
}
/// Their phases, consecutive repeats folded: what the page's hero went through on events alone (review M1).
fn hero_phases(seen: &[Seen]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in job_events(seen) {
        let phase = e["phase"].as_str().unwrap_or("").to_owned();
        if out.last() != Some(&phase) {
            out.push(phase);
        }
    }
    out
}
fn strs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// The rows of a failed job carry the outcome's error, rendered in `lang`.
fn assert_rows_failed(s: &Value, ids: &[&str], key: &str, lang: Lang, what: &str) {
    let expected: Vec<(&str, &str)> = ids.iter().map(|id| (*id, "failed")).collect();
    assert_eq!(rows(s), pairs(&expected), "{what}");
    for row in s["rows"].as_array().into_iter().flatten() {
        assert_eq!(row["error"]["key"], key, "{what}");
        assert_eq!(row["error"]["message"], text(lang, key), "{what}");
    }
}

// ------------------------------------------------------------------------------------------------ calls

/// What a scenario expects of the fake since its `Load()` and of the stub log since it began.
#[derive(Debug, Default)]
struct Expect {
    /// the PackageKit calls, in order (method labels of `CallsJson()`)
    pk: Vec<String>,
    /// NetworkManager property reads (`Properties.Get`, always NO_AUTO_START)
    nm: usize,
    /// `Notify` calls on the fake notification server (session bus)
    notify: usize,
    /// phasing classifier runs: the `python3` tool stub, `-I -c <script>`
    stubs: usize,
}
fn tx(method: &str) -> Vec<String> {
    ["CreateTransaction", "SetHints", method].iter().map(|m| (*m).to_owned()).collect()
}
fn seq(parts: &[Vec<String>]) -> Vec<String> {
    parts.concat()
}
fn one(method: &str) -> Vec<String> {
    vec![method.to_owned()]
}
/// RefreshCache, GetUpdates and GetDetails (sizes for the offered packages).
fn check_calls() -> Vec<String> {
    seq(&[tx("RefreshCache"), tx("GetUpdates"), tx("GetDetails")])
}
/// A check that finds nothing offered: no GetDetails.
fn check_calls_bare() -> Vec<String> {
    seq(&[tx("RefreshCache"), tx("GetUpdates")])
}
/// The simulate and the real UpdatePackages.
fn install_calls() -> Vec<String> {
    seq(&[tx("UpdatePackages"), tx("UpdatePackages")])
}

/// Compare the fake's calls and the stub log with `exp`; the counts may settle for up to 3 s (a Notify or a last
/// reply can still be on its way). Also checks the call arguments of design §4.6 (fixed flags, the simulate's
/// ids, NO_AUTO_START reads) and that `State().violations` is empty. Returns the calls.
fn verify(fake: &Fake, stub_mark: usize, exp: &Expect, what: &str) -> Vec<Value> {
    let label = |c: &Value| c["method"].as_str().unwrap_or("").to_owned();
    let count = |calls: &[Value], to: &str| calls.iter().filter(|c| c["to"] == to).count();
    let started = Instant::now();
    let calls = loop {
        let calls = fake.calls();
        let pk: Vec<String> = calls.iter().filter(|c| c["to"] == "pk").map(label).collect();
        if (pk == exp.pk && count(&calls, "nm") == exp.nm && count(&calls, "notify") == exp.notify)
            || started.elapsed() > Duration::from_secs(3)
        {
            break calls;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let pk: Vec<String> = calls.iter().filter(|c| c["to"] == "pk").map(label).collect();
    assert_eq!(pk, exp.pk, "{what}: the PackageKit calls");
    assert_eq!(count(&calls, "nm"), exp.nm, "{what}: NetworkManager reads");
    assert_eq!(count(&calls, "notify"), exp.notify, "{what}: notifications");
    assert_eq!(fake.call_count(), calls.len(), "{what}: CallCount() = CallsJson()");
    let mut simulated: Option<Value> = None;
    for c in &calls {
        let method = label(c);
        if c["to"] == "nm" {
            assert_eq!((method.as_str(), c["noAutoStart"].clone()), ("Properties.Get", json!(true)), "{what}: NM read {c}");
        }
        match method.as_str() {
            "GetTransactionList" | "Properties.GetAll" | "Properties.Get" if c["to"] == "pk" => {
                assert_eq!(c["noAutoStart"], true, "{what}: {method} must not start packagekitd")
            }
            "RefreshCache" => assert_eq!(c["args"], json!([false]), "{what}"),
            "GetUpdates" => assert_eq!(c["args"], json!([2]), "{what}: filter none"),
            "RepairSystem" => assert_eq!(c["args"], json!([2]), "{what}: ONLY_TRUSTED"),
            "GetOldTransactions" => assert_eq!(c["args"], json!([5]), "{what}"),
            "UpdatePackages" => match c["args"][0].as_u64() {
                Some(6) => simulated = Some(c["args"][1].clone()),
                Some(2) => assert_eq!(Some(&c["args"][1]), simulated.as_ref(), "{what}: the real install sends the simulated ids"),
                flags => panic!("{what}: UpdatePackages with flags {flags:?}"),
            },
            "SetHints" => {
                let hints = c["args"][0].as_array().cloned().unwrap_or_default();
                assert!(hints.contains(&json!("interactive=true")) && hints.contains(&json!("background=false")), "{what}: hints {hints:?}")
            }
            _ => {}
        }
    }
    let stubs: Vec<Value> = fake.stub_log().into_iter().skip(stub_mark).collect();
    assert_eq!(stubs.len(), exp.stubs, "{what}: stub calls {}", short(&json!(stubs)));
    for s in &stubs {
        assert_eq!((s["tool"].as_str(), s["dir"].as_str(), s["rc"].as_i64()), (Some("python3"), Some("tools"), Some(0)), "{what}: {s}");
        let argv = s["argv"].as_array().cloned().unwrap_or_default();
        assert_eq!(argv.get(..2), Some(&[json!("-I"), json!("-c")][..]), "{what}: the classifier script");
    }
    assert_eq!(fake.state()["violations"], json!([]), "{what}: fake violations");
    calls
}
/// The ids of the last `method` call in `calls`.
fn ids_of(calls: &[Value], method: &str) -> Vec<String> {
    calls
        .iter()
        .rev()
        .find(|c| c["method"] == method)
        .and_then(|c| c["args"].as_array()?.last()?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

// ------------------------------------------------------------------------------------------------ timelines

/// Every timeline but `flood` (that one is [`progress_under_the_flood_timeline`], in both languages too).
const SCENARIOS: [&str; 31] = [
    "idle-none",
    "available-mixed",
    "only-phased",
    "phased-as-normal",
    "refresh-partial-fail",
    "refresh-offline",
    "update-details",
    "updates-changed",
    "daemon-absent",
    "daemon-idle-activatable",
    "download-install",
    "cancel-download",
    "cannot-cancel",
    "auth-wait",
    "lock-wait",
    "not-authorized",
    "no-space",
    "untrusted",
    "conflict",
    "install-blocked",
    "cancelled-priority",
    "dpkg-fail-halfway",
    "success-rows-remain",
    "removals-in-simulate",
    "metered",
    "require-restart-system",
    "repair-required",
    "foreign-transaction",
    "caller-vanishes-reattach",
    "finished-in-gap",
    "vanish-cancels-download",
];

/// One timeline through the real service in `lang` (design §9.2: every GW1 timeline in nl and en).
fn timeline(fake: &Fake, name: &str, lang: Lang) {
    let what = format!("{name} ({})", lang.code());
    let scn = Scenario::new();
    let mark = fake.stub_log().len();
    let t = |key: &str| text(lang, key);
    if name == "foreign-transaction" {
        // the service must be running before the foreign transaction appears: start it on the defaults first
        fake.load("");
    } else {
        fake.load(name);
        fake.run();
    }
    match name {
        "idle-none" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            assert_eq!(s["phase"], "idle", "{what}");
            assert_eq!((s["rows"].clone(), s["totals"]["count"].clone(), s["error"].clone()), (json!([]), json!(0), Value::Null));
            assert_eq!(s["phasing"], "apt");
            verify(fake, mark, &Expect { pk: check_calls_bare(), nm: 4, ..Default::default() }, &what);
        }
        "available-mixed" => {
            let h = Host::owner(&scn, "a", lang);
            assert_eq!(h.call("update.check", json!({})).unwrap()["started"], true);
            // review F16: an install while the check runs is refused and starts no transaction (WU-FIX, GW2b
            // review n6: with its own text, not the running job's update.err.busy)
            assert_error(&h.call("update.install", json!({})).unwrap_err(), "busy", "update.err.checking", lang, &what);
            let s = h.checked();
            assert_eq!(s["phase"], "available", "{what}");
            let ids: Vec<&str> = s["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
            assert_eq!(ids, ["security", "gnome", "webkit", "gtk", "soup", "glib", "kernel", "other"], "{what}: design §3.3 order");
            let counts: Vec<u64> = s["rows"].as_array().unwrap().iter().map(|r| r["count"].as_u64().unwrap()).collect();
            assert_eq!(counts, [4, 4, 2, 2, 2, 3, 3, 2]);
            let versions: Vec<Value> = s["rows"].as_array().unwrap().iter().map(|r| r["version"].clone()).collect();
            assert_eq!(versions, [json!("26.04.1 LTS"), json!("50.1"), json!("2.52.7"), json!("3.24.52"), json!("3.6.6"), json!("2.88.0"), json!("7.0.0.43.43"), Value::Null]);
            assert!(s["rows"].as_array().unwrap().iter().all(|r| r["state"] == "pendingDownload"));
            assert_eq!((s["totals"]["count"].clone(), s["totals"]["security"].clone(), s["totals"]["phased"].clone(), s["totals"]["held"].clone()), (json!(22), json!(11), json!(2), json!(0)));
            assert_eq!(s["phasing"], "apt");
            let p = h.call("update.status", json!({"packages":true})).unwrap();
            let packages = p["packages"].as_array().unwrap();
            assert_eq!(packages.len(), 24);
            let phased: Vec<&str> = packages.iter().filter(|p| p["phased"] == true).map(|p| p["name"].as_str().unwrap()).collect();
            assert_eq!(phased, ["ubuntu-drivers-common", "fwupd"], "{what}: apt's phasing rule from the classifier");
            let soup = packages.iter().find(|p| p["name"] == "libsoup-3.0-0").unwrap();
            assert_eq!((soup["from"].clone(), soup["to"].clone(), soup["row"].clone()), (json!("3.6.6-1ubuntu0.2"), json!("3.6.6-1ubuntu0.3"), json!("soup")));
            let calls = verify(fake, mark, &Expect { pk: check_calls(), nm: 4, stubs: 1, ..Default::default() }, &what);
            let details = ids_of(&calls, "GetDetails");
            assert_eq!(details.len(), 22, "{what}: sizes only for offered packages");
            assert!(!details.iter().any(|id| id.starts_with("fwupd;") || id.starts_with("ubuntu-drivers-common;")));
            let stdin = fake.stub_log()[mark]["stdin"].as_str().unwrap_or("").to_owned();
            assert_eq!(stdin.lines().count(), 24, "{what}: every name once on stdin");
            assert!(stdin.contains("libsoup-3.0-0\tamd64\n") && stdin.contains("tzdata\tall\n"));
        }
        "only-phased" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            assert_eq!((s["phase"].clone(), s["rows"].clone()), (json!("idle"), json!([])), "{what}: only phased is up to date");
            assert_eq!((s["totals"]["count"].clone(), s["totals"]["phased"].clone()), (json!(0), json!(2)));
            verify(fake, mark, &Expect { pk: check_calls_bare(), nm: 4, stubs: 1, ..Default::default() }, &what);
        }
        "phased-as-normal" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            assert_eq!(rows(&s), pairs(&[("security", "pendingDownload"), ("other", "pendingDownload")]), "{what}");
            assert_eq!(s["totals"]["phased"], 1);
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!(s["phase"], "idle", "{what}");
            assert_eq!(rows(&s), pairs(&[("security", "installed"), ("other", "installed")]));
            // review m1: the simulate reports 0 (unknown), so the page first has no total and then the download's
            // first DownloadSizeRemaining (the 3 sent packages), through update.changed, never 0
            let totals: Vec<Value> = job_events(&h.seen()).iter().map(|e| e["totals"]["downloadSize"].clone()).collect();
            assert!(!totals.contains(&json!(0)), "{what}: a total of 0 reached the page: {totals:?}");
            assert_eq!(totals.first(), Some(&Value::Null), "{what}: {totals:?}");
            assert_eq!(totals.last(), Some(&json!(9_732_096)), "{what}: the download's total {totals:?}");
            assert!(job_events(&h.seen()).iter().any(|e| e["phase"] == "downloading" && e["totals"]["downloadSize"] == 9_732_096 && e["job"]["downloadSize"] == 9_732_096), "{what}");
            assert_eq!((s["totals"]["count"].clone(), s["totals"]["phased"].clone()), (json!(0), json!(1)));
            let calls = verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 2 }, &what);
            let sent = ids_of(&calls, "UpdatePackages");
            assert_eq!(sent.len(), 3);
            assert!(!sent.iter().any(|id| id.starts_with("ubuntu-drivers-common;")), "{what}: a phased id reached UpdatePackages");
        }
        "refresh-partial-fail" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            assert_eq!((s["phase"].clone(), s["error"].clone()), (json!("available"), Value::Null), "{what}");
            let warning = &s["warnings"][0];
            assert_eq!(warning["key"], "update.warn.source");
            let params = json!({"source":"dl.google.com","product":i18n::tr(lang, "update.product")});
            assert_eq!(warning["message"], i18n::trf(lang, "update.warn.source", &params), "{what}");
            assert_eq!(s["warnings"].as_array().unwrap().len(), 1);
            verify(fake, mark, &Expect { pk: check_calls(), nm: 4, stubs: 1, ..Default::default() }, &what);
        }
        "refresh-offline" => {
            let h = Host::owner(&scn, "a", lang);
            assert_error(&h.call("update.check", json!({})).unwrap_err(), "offline", "update.err.offline", lang, &what);
            let s = h.status();
            assert_eq!(s["phase"], "offline", "{what}");
            assert_eq!(s["network"], json!({"state":"none","online":false,"metered":false}));
            verify(fake, mark, &Expect { nm: 4, ..Default::default() }, &what);
        }
        "update-details" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            let sizes: Vec<(String, Value)> = s["rows"].as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap().to_owned(), r["size"].clone())).collect();
            assert_eq!(sizes, [("webkit".to_owned(), json!(21734410)), ("soup".to_owned(), json!(336532)), ("other".to_owned(), json!(274808))], "{what}: GetDetails download-size");
            assert_eq!(s["totals"]["downloadSize"], 22345750);
            let ids = h.call("update.status", json!({"packages":true})).unwrap()["packages"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect::<Vec<_>>();
            let details = h.call("update.details", json!({"ids":ids})).expect("details");
            let list = details["details"].as_array().unwrap();
            assert_eq!(list.len(), 3, "{what}");
            let soup = list.iter().find(|d| d["id"].as_str().unwrap().starts_with("libsoup-3.0-0;")).unwrap();
            assert_eq!(soup["updateText"], "Security fixes for libsoup.");
            assert_eq!(soup["cves"], json!(["https://www.cve.org/CVERecord?id=CVE-2026-2443", "https://www.cve.org/CVERecord?id=CVE-2026-2784"]));
            assert!(soup["changelog"].as_str().unwrap().contains("CVE-2026-2443"));
            let webkit = list.iter().find(|d| d["id"].as_str().unwrap().starts_with("libwebkit2gtk-4.1-0;")).unwrap();
            assert_eq!((webkit["bugs"].clone(), webkit["restart"].clone()), (json!(["https://launchpad.net/bugs/2131047"]), json!(2)));
            // cached: the same ids again make no call
            assert_eq!(h.call("update.details", json!({"ids":ids})).expect("cached details")["details"].as_array().unwrap().len(), 3);
            assert_error(&h.call("update.details", json!({"ids":["zlib1g;1:1.3-1;amd64;ubuntu-resolute-updates"]})).unwrap_err(), "stale", "update.err.stale", lang, &what);
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), tx("GetUpdateDetail")]), stubs: 1, ..Default::default() }, &what);
        }
        "updates-changed" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            assert_eq!((s["totals"]["count"].clone(), rows(&s)), (json!(2), pairs(&[("soup", "pendingDownload")])), "{what}");
            let (checked, rev) = (s["checkedAt"].as_u64().unwrap(), s["rev"].as_u64().unwrap());
            // apt installed libsoup-3.0-common elsewhere and its 20packagekit hook (the test plays it) says so:
            // UpdatesChanged → the owner rescans and publishes, and once the signals settled (2 s) reads the list
            // again without RefreshCache (review m2), so the installed package leaves the page
            fake.pk_call(PK_PATH, PK, "StateHasChanged", Some(("cache-update",).to_variant())).expect("StateHasChanged");
            h.wait("a new rev after UpdatesChanged", LONG, |s| s["rev"].as_u64() > Some(rev));
            let s = h.wait("the list read again", LONG, |s| s["totals"]["count"] == 1 && s["refreshing"] == false);
            assert_eq!((s["phase"].clone(), s["rows"][0]["count"].clone()), (json!("available"), json!(1)), "{what}");
            assert!(s["checkedAt"].as_u64().unwrap() > checked, "{what}: the list was read again");
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), one("StateHasChanged"), tx("GetUpdates"), tx("GetDetails")]), nm: 4, stubs: 2, ..Default::default() }, &what);
        }
        "daemon-absent" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.status();
            assert_eq!((s["phase"].clone(), s["backend"].clone()), (json!("unsupported"), Value::Null), "{what}");
            assert_snapshot_error(&s, "update.err.noPackageKit", lang, &what);
            assert_error(&h.call("update.check", json!({})).unwrap_err(), "unsupported", "update.err.noPackageKit", lang, &what);
            assert_error(&h.call("update.install", json!({})).unwrap_err(), "unsupported", "update.err.noPackageKit", lang, &what);
            verify(fake, mark, &Expect { nm: 2, ..Default::default() }, &what);
            assert_eq!(fake.state()["activations"], 0);
        }
        "daemon-idle-activatable" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.status();
            assert_eq!((s["phase"].clone(), s["backend"].clone()), (json!("idle"), json!("packagekit")), "{what}: installed but idle");
            assert_eq!(fake.state()["daemon"], false, "{what}: nothing started packagekitd before the check");
            let s = h.check();
            assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload")]), "{what}");
            let state = fake.state();
            assert_eq!((state["activations"].clone(), state["daemon"].clone()), (json!(1), json!(true)), "{what}: the check activated it");
            verify(fake, mark, &Expect { pk: check_calls(), nm: 4, stubs: 1, ..Default::default() }, &what);
        }
        "download-install" => {
            let h = Host::owner(&scn, "a", lang);
            let s = h.check();
            let total = 134_545_408u64;
            h.install(json!({}));
            assert!(h.held(), "{what}: the keep-alive is held while the transaction runs");
            let s2 = h.done(&what);
            assert_eq!(s2["phase"], "idle", "{what}");
            assert_eq!(rows(&s2), pairs(&[("webkit", "installed"), ("gtk", "installed"), ("other", "installed")]));
            // review F2/F3 through what the host saw at every update.changed
            let seen = h.seen();
            assert_eq!(s["rows"].as_array().unwrap().len(), 3, "{what}: webkit, gtk and other were offered");
            let first = seen.iter().position(|v| v.snapshot["lastJob"].is_object()).expect("a snapshot after Finished");
            let after = &seen[first];
            assert_eq!(after.snapshot["checking"], true, "{what}: written before the re-check answered");
            assert_eq!(after.snapshot["packages"], json!([]), "{what}: the installed packages left snapshot.json at once");
            assert!(after.held, "{what}: still held until the re-check answered");
            let last = seen.last().unwrap();
            assert_eq!((last.snapshot["checking"].clone(), last.held), (json!(false), false), "{what}: the hold ends after the re-check");
            assert!(seen[..first].iter().any(|v| v.snapshot["downloadSize"] == total && v.snapshot["totals"]["downloadSize"] == total), "{what}: totals.downloadSize during the download");
            // review M1: the hero's phases reach the page as update.changed, before Finished
            assert_eq!(hero_phases(&seen[..first]), strs(&["downloading", "installing"]), "{what}: the phases on update.changed");
            assert!(job_events(&seen[..first]).iter().any(|e| e["phase"] == "installing" && e["job"]["committing"] == true), "{what}");
            let progress = h.progress.lock().unwrap().clone();
            assert!(!progress.is_empty() && progress.iter().all(|(_, p)| p["jobId"].as_str().is_some_and(store::is_token)), "{what}: progress events of the job");
            let file = scn.store().read(store::JOB).unwrap();
            assert_eq!((file["outcome"]["exit"].clone(), file["finishedAt"].is_u64()), (json!(1), true));
            assert_eq!(scn.store().jobs().len(), 1);
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 1 }, &what);
        }
        "cancel-download" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            let job = h.install(json!({}));
            let s = h.wait("the cancellable download", LONG, |s| s["job"]["status"] == 8 && s["job"]["allowCancel"] == true);
            assert_eq!(s["phase"], "downloading", "{what}");
            assert_eq!(h.call("update.cancel", json!({"jobId":job})).expect("cancel"), json!({"ok":true}));
            let s = h.done(&what);
            assert_eq!((s["phase"].clone(), s["error"].clone()), (json!("available"), Value::Null), "{what}");
            assert_eq!(s["warnings"][0]["key"], "update.err.cancelled");
            assert_eq!(s["warnings"][0]["message"], t("update.err.cancelled"));
            assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload"), ("other", "pendingDownload")]), "{what}: nothing half-installed");
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), one("Cancel"), tx("GetUpdates"), tx("GetDetails")]), nm: 4, notify: 0, stubs: 2 }, &what);
        }
        "cannot-cancel" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            let job = h.install(json!({}));
            let s = h.wait("the commit phase", LONG, |s| s["job"]["committing"] == true && s["job"]["allowCancel"] == false);
            assert_eq!(s["phase"], "installing", "{what}");
            // refused here, without a Cancel call on the bus (design §4.7)
            assert_error(&h.call("update.cancel", json!({"jobId":job})).unwrap_err(), "cannot_cancel", "update.err.cannotCancel", lang, &what);
            // the timeline waits for a Cancel call: the test is that second client, and PackageKit refuses it
            let tid = scn.store().read(store::JOB).unwrap()["tid"].as_str().unwrap().to_owned();
            let refused = fake.pk_call(&tid, TX_IFACE, "Cancel", None).unwrap_err();
            assert!(refused.message().contains("CannotCancel"), "{what}: {refused}");
            let s = h.done(&what);
            assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("webkit", "installed"), ("other", "installed")])), "{what}");
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), one("Cancel"), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 1 }, &what);
        }
        "auth-wait" | "lock-wait" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            // lock-wait in nl: unattended-upgrades holds the lock (fixture procfs); in en another program
            let unattended = name == "lock-wait" && lang == Lang::Nl;
            if unattended {
                write(&fake.fixture.join("proc/4242/comm"), "unattended-upgr\n");
            }
            h.install(json!({}));
            let status = if name == "auth-wait" { 31 } else { 30 };
            let s = h.wait("the wait", LONG, |s| s["job"]["status"] == status);
            assert_eq!(s["phase"], "waiting", "{what}");
            let holder = if unattended { json!("unattended") } else { Value::Null };
            assert_eq!(s["job"]["lockHolder"], holder, "{what}");
            let s = h.done(&what);
            assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("soup", "installed")])), "{what}");
            // review M1: the page learns of the wait (H07 lock, waitLockAuto / H08 auth) through update.changed alone
            let seen = h.seen();
            assert_eq!(hero_phases(&seen), strs(&["downloading", "waiting", "downloading", "installing"]), "{what}");
            let waits: Vec<Value> = job_events(&seen).into_iter().filter(|e| e["phase"] == "waiting").collect();
            assert_eq!(waits.len(), 1, "{what}: one update.changed for the wait, not one per PropertiesChanged");
            assert_eq!((waits[0]["job"]["status"].clone(), waits[0]["job"]["lockHolder"].clone()), (json!(status), holder), "{what}");
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 1 }, &what);
        }
        "not-authorized" | "no-space" | "untrusted" | "install-blocked" | "cancelled-priority" | "conflict" => {
            let (key, detail, row_ids): (&str, &str, &[&str]) = match name {
                "not-authorized" => ("update.err.notAuthorized", "Failed to obtain authentication.", &["soup"]),
                "no-space" => ("update.err.noSpace", "You don't have enough free space in /var/cache/apt/archives/.", &["webkit", "other"]),
                "untrusted" => ("update.err.untrusted", "", &["webkit", "other"]),
                "install-blocked" => ("update.err.conflict", "blocked from installation", &["webkit", "other"]),
                "cancelled-priority" => ("update.err.locked", "cancelled by a higher priority task", &["webkit", "other"]),
                _ => ("update.err.conflict", "unmet dependencies", &["webkit", "other"]),
            };
            let h = Host::owner(&scn, "a", lang);
            h.check();
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!(s["phase"], "error", "{what}");
            assert_snapshot_error(&s, key, lang, &what);
            assert!(s["error"]["detail"].as_str().unwrap().contains(detail), "{what}: detail {}", s["error"]["detail"]);
            assert_rows_failed(&s, row_ids, key, lang, &what);
            let install = if name == "conflict" { tx("UpdatePackages") } else { install_calls() };
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install, tx("GetUpdates"), tx("GetDetails")]), nm: 4, notify: 1, stubs: 2 }, &what);
        }
        "dpkg-fail-halfway" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!(s["phase"], "error", "{what}");
            assert_snapshot_error(&s, "update.err.dpkg", lang, &what);
            assert!(s["error"]["detail"].as_str().unwrap().contains("libgtk-3-0t64:amd64"));
            assert_eq!(rows(&s), pairs(&[("gtk", "installed"), ("glib", "failed"), ("other", "failed")]), "{what}");
            let calls = verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates"), tx("GetDetails")]), nm: 4, notify: 1, stubs: 2 }, &what);
            let sent = ids_of(&calls, "UpdatePackages");
            assert!(sent[0].starts_with("libgtk-3-0t64;") && sent[1].starts_with("libgtk-3-common;"), "{what}: rows in design order {sent:?}");
        }
        "success-rows-remain" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!((s["phase"].clone(), s["error"].clone(), s["warnings"].clone()), (json!("available"), Value::Null, json!([])), "{what}");
            assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload"), ("other", "installed")]), "{what}: the kept-back package is offered again");
            assert_eq!(s["rows"][0]["count"], 1);
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates"), tx("GetDetails")]), nm: 4, notify: 1, stubs: 2 }, &what);
        }
        "removals-in-simulate" | "metered" => {
            let h = Host::owner(&scn, "a", lang);
            if name == "metered" {
                assert_eq!(h.status()["network"]["metered"], true, "{what}");
            }
            h.check();
            let job = h.install(json!({}));
            let s = h.wait("the confirm", LONG, |s| s["job"]["confirm"].is_object());
            assert_eq!((s["phase"].clone(), s["job"]["simulating"].clone()), (json!("downloading"), json!(false)), "{what}");
            if name == "metered" {
                assert_eq!(s["job"]["confirm"], json!({"kind":"metered","size":29074626}), "{what}");
                assert_eq!(s["totals"]["downloadSize"], 29074626);
            } else {
                assert_eq!(s["job"]["confirm"], json!({"kind":"removals","removals":["libwebkit2gtk-4.0-37","libjavascriptcoregtk-4.0-18","tzdata-legacy"]}), "{what}");
            }
            // review F7: nothing runs while the user decides, so nothing is held
            assert!(!h.held(), "{what}: no keep-alive during a confirm");
            let mut nm = 4;
            if name == "metered" {
                // the gate of the timeline: NM turns GUESS_YES (still metered: one read of both, no change)
                assert!(fake.step().contains("released"), "{what}");
                wait_for("the NetworkManager re-read", LONG, || fake.calls().iter().filter(|c| c["to"] == "nm").count() == 6);
                nm = 6;
            }
            let accept = name == "metered" || lang == Lang::Nl;
            assert_eq!(h.call("update.confirm", json!({"jobId":job,"accept":accept})).expect("confirm"), json!({"ok":true}));
            if accept {
                assert!(h.held(), "{what}: accept holds again");
                let s = h.done(&what);
                let installed = if name == "metered" { pairs(&[("webkit", "installed")]) } else { pairs(&[("webkit", "installed"), ("other", "installed")]) };
                assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), installed), "{what}");
                verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm, notify: 1, stubs: 1 }, &what);
            } else {
                // declined: nothing was installed, no re-check, no notification, no error
                let s = h.done(&what);
                assert_eq!((s["phase"].clone(), s["error"].clone(), s["warnings"].clone()), (json!("available"), Value::Null, json!([])), "{what}");
                assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload"), ("other", "pendingDownload")]));
                assert!(!h.held());
                verify(fake, mark, &Expect { pk: seq(&[check_calls(), tx("UpdatePackages")]), nm, notify: 0, stubs: 1 }, &what);
            }
        }
        "require-restart-system" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!(s["phase"], "restartRequired", "{what}");
            assert_eq!(s["restart"]["reboot"], true);
            assert_eq!(s["restart"]["rebootPkgs"], json!(["linux-image-7.0.0-43-generic", "linux-base", "linux-image-generic"]));
            assert_eq!(rows(&s), pairs(&[("security", "installed"), ("kernel", "restart")]), "{what}");
            verify(fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 1 }, &what);
        }
        "repair-required" => {
            let h = Host::owner(&scn, "a", lang);
            h.check();
            h.install(json!({}));
            let s = h.done(&what);
            assert_eq!(s["phase"], "repairRequired", "{what}");
            assert_snapshot_error(&s, "update.err.repair", lang, &what);
            assert_eq!(scn.store().read(store::SNAPSHOT).unwrap()["repairRequired"], true);
            let job = h.call("update.repair", json!({})).expect("repair");
            assert!(job["jobId"].as_str().is_some_and(store::is_token));
            let s = h.done(&what);
            // review F14: a success clears repairRequired (and the failed outcome's error)
            assert_eq!(scn.store().read(store::SNAPSHOT).unwrap()["repairRequired"], false, "{what}");
            assert_eq!((s["phase"].clone(), s["error"].clone()), (json!("available"), Value::Null), "{what}");
            assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload"), ("other", "pendingDownload")]));
            verify(
                fake,
                mark,
                &Expect {
                    pk: seq(&[check_calls(), install_calls(), tx("GetUpdates"), tx("GetDetails"), tx("RepairSystem"), tx("GetUpdates"), tx("GetDetails")]),
                    nm: 4,
                    notify: 2,
                    stubs: 3,
                },
                &what,
            );
        }
        "foreign-transaction" => {
            // nl: the foreign transaction is our own Software installer (a live Software job file); en: another program
            let own = lang == Lang::Nl;
            if own {
                let jobs = scn.cache().join("software/jobs");
                fs::create_dir_all(&jobs).unwrap();
                store::atomic(&jobs.join("j1.json"), json!({"state":"running","_owner":store::current_owner()}).to_string().as_bytes()).unwrap();
            }
            let h = Host::owner(&scn, "a", lang);
            fake.load(name);
            fake.run();
            let s = h.wait("the foreign transaction", LONG, |s| s["phase"] == "foreign");
            assert_eq!((s["job"]["foreign"].clone(), s["job"]["foreignOwn"].clone(), s["job"]["jobId"].clone()), (json!(true), json!(own), Value::Null), "{what}");
            assert_error(&h.call("update.install", json!({})).unwrap_err(), "locked", "update.err.locked", lang, &what);
            h.wait("the foreign transaction's end", LONG, |s| s["phase"] == "idle" && s["job"].is_null());
            verify(fake, mark, &Expect { pk: one("Properties.Get"), ..Default::default() }, &what);
        }
        "caller-vanishes-reattach" | "finished-in-gap" | "vanish-cancels-download" => {
            let a = Host::owner(&scn, "a", lang);
            a.check();
            a.install(json!({}));
            wait_for("the transaction at its vanish gate", LONG, || fake.armed("vanish"));
            let took = a.shutdown();
            eprintln!("{what}: shutdown during the transaction took {took:?}");
            assert!(took < SHUTDOWN_LIMIT, "{what}: shutdown took {took:?} while a transaction ran");
            if name == "caller-vanishes-reattach" {
                wait_for("the caller's vanish", LONG, || fake.transactions().iter().any(|t| t["vanished"] == true));
            } else {
                // the transaction ends in the gap (finished, or cancelled because its caller left)
                wait_for("the transaction's end in the gap", LONG, || fake.transactions().is_empty());
            }
            let b = Host::owner(&scn, "b", lang);
            let mut expect_pk = seq(&[check_calls(), install_calls(), one("GetTransactionList")]);
            match name {
                "caller-vanishes-reattach" => {
                    let s = b.wait("the reattached job", LONG, |s| s["job"]["reattached"] == true);
                    assert_eq!(s["job"]["role"], "update", "{what}");
                    let s = b.done(&what);
                    assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("webkit", "installed"), ("other", "installed")])), "{what}");
                    expect_pk = seq(&[expect_pk, one("Properties.GetAll"), tx("GetUpdates")]);
                    verify(fake, mark, &Expect { pk: expect_pk, nm: 6, notify: 1, stubs: 1 }, &what);
                }
                "finished-in-gap" => {
                    let s = b.done(&what);
                    assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("webkit", "installed"), ("other", "installed")])), "{what}: GetOldTransactions says it succeeded");
                    expect_pk = seq(&[expect_pk, tx("GetOldTransactions"), tx("GetUpdates")]);
                    verify(fake, mark, &Expect { pk: expect_pk, nm: 6, notify: 1, stubs: 1 }, &what);
                }
                _ => {
                    let s = b.done(&what);
                    assert_eq!(s["phase"], "error", "{what}");
                    assert_snapshot_error(&s, "update.err.failed", lang, &what);
                    assert_rows_failed(&s, &["webkit", "other"], "update.err.failed", lang, &what);
                    expect_pk = seq(&[expect_pk, tx("GetOldTransactions"), tx("GetUpdates"), tx("GetDetails")]);
                    verify(fake, mark, &Expect { pk: expect_pk, nm: 6, notify: 1, stubs: 2 }, &what);
                }
            }
            let file = scn.store().read(store::JOB).unwrap();
            assert!(file["finishedAt"].is_u64(), "{what}: the next owner ended the job");
        }
        other => panic!("no scenario for the timeline {other}"),
    }
}

/// Every GW1 timeline (32, `tests/update/timelines/*.json`) through the real service, in nl and en; `flood` is
/// [`progress_under_the_flood_timeline`]. Covers the review's engine paths too: an install during a check is
/// `update.err.checking` and starts nothing (F16, WU-FIX n6); after `Finished` the job's packages are gone from snapshot.json
/// before the re-check answers, the rows are `installed`/`restart`/`failed`, and the keep-alive ends after the
/// re-check (F2, F3); a confirm pause holds nothing and accept holds again (F7); a success clears
/// `repairRequired` (F14); reattach after the owner left; a foreign transaction, also our own (foreignOwn); cancel
/// in the download phase. All scenarios run; the failures are reported together.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn every_timeline_in_nl_and_en() {
    let fake = Fake::new();
    let mut files: Vec<String> = fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/timelines"))
        .expect("tests/update/timelines")
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
        .collect();
    files.sort();
    let mut covered: Vec<String> = SCENARIOS.iter().map(|s| (*s).to_owned()).chain(["flood".to_owned()]).collect();
    covered.sort();
    assert_eq!(files, covered, "every timeline has a scenario");
    assert_eq!(files.len(), 32);
    let mut failures = Vec::new();
    for name in SCENARIOS {
        for lang in Lang::ALL {
            let started = Instant::now();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| timeline(&fake, name, lang)));
            let verdict = if result.is_ok() { "ok" } else { "FAILED" };
            eprintln!("bus scenario {name} ({}): {verdict} in {:.2} s", lang.code(), started.elapsed().as_secs_f64());
            if let Err(panic) = result {
                let message = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_default();
                failures.push(format!("{name} ({}): {message}", lang.code()));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {} scenarios failed:\n{}", failures.len(), SCENARIOS.len() * 2, failures.join("\n"));
}

/// Under the flood timeline (200 ItemProgress + 200 PropertiesChanged per second for 60 s), in nl and en: the
/// owner's own page gets at most 4 `update.progress` per second (`emit_local`, every delivery counted by its
/// observer), and a second host's Events in the same runtime directory receives at most 1 per second over the
/// real datagram fan-out (design §4.4, review F8). The install then ends as usual.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn progress_under_the_flood_timeline() {
    let fake = Fake::new();
    for lang in Lang::ALL {
        let what = format!("flood ({})", lang.code());
        let scn = Scenario::new();
        let mark = fake.stub_log().len();
        fake.load("flood");
        fake.run();
        let events = scn.dir.join("events");
        let h = Host::new(&scn, "a", "settings", lang, false, Some(&events));
        assert_eq!(h.service.mode(), Mode::Owner);
        let remote = Events::new(&events).unwrap();
        let arrived: Log<Instant> = Default::default();
        {
            let arrived = arrived.clone();
            remote.on("update.progress", Arc::new(move |_: &Value| arrived.lock().unwrap().push(Instant::now())));
        }
        h.check();
        h.install(json!({}));
        let s = h.wait("the end of the flood", Duration::from_secs(150), |s| s["job"].is_null() && s["refreshing"] == false);
        assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("soup", "installed")])), "{what}");
        let local: Vec<Instant> = h.progress.lock().unwrap().iter().map(|(at, _)| *at).collect();
        let sends: Vec<Instant> = h.service.shared.as_ref().unwrap().stats.fanout.lock().unwrap().clone();
        wait_for("the last fan-out datagram", Duration::from_secs(3), || arrived.lock().unwrap().len() >= sends.len());
        let wire = arrived.lock().unwrap().clone();
        let most_in = |times: &[Instant], window: Duration| {
            (0..times.len()).map(|i| times[i..].iter().take_while(|t| t.duration_since(times[i]) < window).count()).max().unwrap_or(0)
        };
        let span = |times: &[Instant]| times.last().zip(times.first()).map(|(l, f)| l.duration_since(*f)).unwrap_or_default();
        eprintln!(
            "{what}: {} page events (at most {} in any second), {} fan-out sends (at most {} in any second, {:.2} s apart at the least), {} arrived (at most {} in any second, {} in {:.1} s)",
            local.len(),
            most_in(&local, Duration::from_secs(1)),
            sends.len(),
            most_in(&sends, Duration::from_secs(1)),
            sends.windows(2).map(|w| w[1].duration_since(w[0]).as_secs_f64()).fold(f64::INFINITY, f64::min),
            wire.len(),
            most_in(&wire, Duration::from_secs(1)),
            wire.len(),
            span(&wire).as_secs_f64()
        );
        assert!(local.len() >= 150, "{what}: {} page events in 60 s", local.len());
        assert!(wire.len() >= 40, "{what}: {} fan-out events in 60 s", wire.len());
        // WU-FIX (item 8): the throttle (model::Progress) keeps at least 250 ms between two page deliveries on its own
        // clock, so 5 of them span at least 1 s there. These times are the observer's: taken after the event is built,
        // so the pk thread's scheduling adds jitter, and under the flood the deliveries come 250-255 ms apart (GW5 saw
        // 5 in one second at load 3.9: a 1 s window then counts 5). On arrival: at most 4 in any 900 ms, and n of them
        // span at least (n-1) x 250 ms less 0.1 s (the same jitter allowance as the fan-out check below).
        assert!(most_in(&local, Duration::from_millis(900)) <= 4, "{what}: more than 4 page events in 900 ms");
        assert!(
            (local.len() as f64 - 1.0) * 0.25 <= span(&local).as_secs_f64() + 0.1,
            "{what}: {} page events in {:?}",
            local.len(),
            span(&local)
        );
        // review m4: at most 1 per second, on the throttle's own clock (no jitter there), and every datagram that
        // arrived is one of those sends
        assert!(most_in(&sends, Duration::from_secs(1)) <= 1, "{what}: more than 1 fan-out send in one second");
        assert_eq!(wire.len(), sends.len(), "{what}: the datagrams that arrived are the throttled sends");
        // on arrival (some jitter): at most one in any 900 ms, and n events span at least n-1 seconds (less 0.1 s)
        assert!(most_in(&wire, Duration::from_millis(900)) <= 1, "{what}: more than 1 fan-out event per second");
        assert!((wire.len() as f64 - 1.0) <= span(&wire).as_secs_f64() + 0.1, "{what}: {} arrivals in {:?}", wire.len(), span(&wire));
        // review M1: under the flood, update.changed only at the structural steps (not per PropertiesChanged)
        let seen = h.seen();
        assert_eq!(hero_phases(&seen), strs(&["downloading", "installing"]), "{what}");
        assert!(job_events(&seen).len() <= 6, "{what}: {} update.changed during the job", job_events(&seen).len());
        remote.shutdown();
        verify(&fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdates")]), nm: 4, notify: 1, stubs: 1 }, &what);
    }
}

/// Two `settings` services on the same fake, cache and runtime directory (design §9.2): one owns, the other
/// mirrors. `update.check` sent to the mirror is forwarded and gives exactly one RefreshCache; the install is
/// forwarded too; `rev` only grows in both; when the owner shuts down during the transaction, the mirror takes
/// over and reattaches to the running job; only the owner (then the former mirror) calls Notifications.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn two_services_one_owner_forwarding_and_takeover() {
    let fake = Fake::new();
    let what = "two services";
    let scn = Scenario::new();
    let mark = fake.stub_log().len();
    fake.load("caller-vanishes-reattach");
    fake.run();
    let events = scn.dir.join("events");
    let o = Host::new(&scn, "o", "settings", Lang::En, false, Some(&events));
    assert_eq!(o.service.mode(), Mode::Owner);
    o.status();
    let m = Host::new(&scn, "m", "settings", Lang::En, false, Some(&events));
    assert_eq!(m.service.mode(), Mode::Mirror);
    let s = m.status();
    assert_eq!((s["mode"].clone(), s["owner"]["mirror"].clone()), (json!("mirror"), json!(true)));
    assert_eq!(fake.calls().iter().filter(|c| c["to"] == "nm").count(), 2, "{what}: only the owner connects");
    let forwarded = |method: &str| {
        let r = m.call(method, json!({})).expect("a forwarded write");
        assert_eq!(r["forwarded"], true, "{what}: {method}");
        let nonce = r["nonce"].clone();
        wait_for("the owner's reply", LONG, || m.replies.lock().unwrap().iter().any(|x| x["nonce"] == nonce));
        let reply = m.replies.lock().unwrap().iter().find(|x| x["nonce"] == nonce).cloned().unwrap();
        assert_eq!(reply["ok"], true, "{what}: {method} {reply}");
    };
    forwarded("update.check");
    m.wait("the forwarded check", LONG, |s| s["checkedAt"].is_u64() && s["refreshing"] == false);
    assert_eq!(fake.calls().iter().filter(|c| c["method"] == "RefreshCache").count(), 1, "{what}: exactly one RefreshCache");
    forwarded("update.install");
    wait_for("the transaction at its vanish gate", LONG, || fake.armed("vanish"));
    let s = m.status();
    assert_eq!((s["phase"].clone(), s["owner"]["mirror"].clone()), (json!("downloading"), json!(true)), "{what}: the mirror reads the owner's job");
    let rev_before = o.status()["rev"].as_u64().unwrap();
    let took = o.shutdown();
    eprintln!("{what}: the owner's shutdown during the transaction took {took:?}");
    assert!(took < SHUTDOWN_LIMIT, "{what}: shutdown took {took:?}");
    wait_for("the mirror's takeover", LONG, || m.service.mode() == Mode::Owner);
    let s = m.wait("the reattached job", LONG, |s| s["job"]["reattached"] == true);
    assert_eq!(s["owner"]["mirror"], false);
    let s = m.done(what);
    assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("webkit", "installed"), ("other", "installed")])), "{what}");
    // rev only grows, in both hosts (each saw the other's update.changed over the fan-out too), and the new owner
    // continued from the old one's snapshot
    let revs = |h: &Host| h.seen().iter().filter_map(|v| v.rev).collect::<Vec<_>>();
    let (ro, rm) = (revs(&o), revs(&m));
    assert!(!ro.is_empty() && !rm.is_empty(), "{what}: both saw update.changed");
    assert!(ro.windows(2).all(|w| w[0] <= w[1]) && rm.windows(2).all(|w| w[0] <= w[1]), "{what}: rev went back: {ro:?} {rm:?}");
    assert!(m.status()["rev"].as_u64().unwrap() > rev_before, "{what}: {rev_before} → {rm:?}");
    assert_eq!(m.service.stats().map(|s| s.3), Some(2), "{what}: two forwarded writes");
    // review M1: the mirror's page followed the owner's job through update.changed (the fan-out)
    assert_eq!(hero_phases(&m.seen()).first().map(String::as_str), Some("downloading"), "{what}");
    let calls = verify(
        &fake,
        mark,
        &Expect { pk: seq(&[check_calls(), install_calls(), one("GetTransactionList"), one("Properties.GetAll"), tx("GetUpdates")]), nm: 6, notify: 1, stubs: 1 },
        what,
    );
    assert_notified_by(&calls, &o, &m, what);
}

/// Review m6: the one `Notify` came from the new owner: the fake's `sender` is the unique name of the session
/// connection the new owner's `notify()` used (GIO's bus singleton is a weak reference, so each `notify()` may get a
/// new one), the old owner sent none, and only the new owner opened the session bus (old: its PackageKit bus once;
/// new: that and the session bus).
fn assert_notified_by(calls: &[Value], old: &Host, new: &Host, what: &str) {
    let notify: Vec<&Value> = calls.iter().filter(|c| c["to"] == "notify").collect();
    assert_eq!(notify.len(), 1, "{what}");
    let by = |h: &Host| h.service.shared.as_ref().map(|s| s.stats.notified.lock().unwrap().clone()).unwrap_or_default();
    assert_eq!((by(old), by(new)), (Vec::<String>::new(), vec![notify[0]["sender"].as_str().unwrap_or("?").to_owned()]), "{what}: the Notify's sender");
    assert_eq!((old.service.stats().map(|s| s.0), new.service.stats().map(|s| s.0)), (Some(1), Some(2)), "{what}: only the new owner notified");
}

/// The owner crashes during a transaction (as other processes see a dead owner: `owner.lock` is free, its bus
/// connection closes, but no `update.owner {gone}` arrives): the mirror takes over through its retry timer on the
/// main loop (design §4.1: 30 s; here 1 s), reattaches to the running job and finishes it, and only it notifies
/// (review m6). The test thread is the main loop (the GTK main thread in the app).
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn a_mirror_takes_over_by_its_retry_after_the_owner_crashed() {
    let fake = Fake::new();
    let what = "takeover by retry";
    let scn = Scenario::new();
    let mark = fake.stub_log().len();
    fake.load("caller-vanishes-reattach");
    fake.run();
    super::MIRROR_RETRY_TEST.store(1, Ordering::SeqCst);
    let main = glib::MainContext::default();
    let pump = || while main.iteration(false) {};
    let o = Host::owner(&scn, "o", Lang::Nl);
    let m = Host::new(&scn, "m", "settings", Lang::Nl, false, None);
    assert_eq!(m.service.mode(), Mode::Mirror);
    o.check();
    o.install(json!({}));
    wait_for("the transaction at its vanish gate", LONG, || fake.armed("vanish"));
    // the retry fires every second, but loses while the owner lives
    let until = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < until {
        pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(m.service.mode(), Mode::Mirror, "{what}: the retry won against a live owner");
    o.service.crash();
    let crashed = Instant::now();
    wait_for("the caller's vanish", LONG, || fake.transactions().iter().any(|t| t["vanished"] == true));
    wait_for("the mirror's retry", Duration::from_secs(10), || {
        pump();
        m.service.mode() == Mode::Owner
    });
    eprintln!("{what}: the mirror took over {:?} after the crash", crashed.elapsed());
    let s = m.wait("the reattached job", LONG, |s| s["job"]["reattached"] == true);
    assert_eq!((s["owner"]["mirror"].clone(), s["job"]["role"].clone()), (json!(false), json!("update")), "{what}");
    let s = m.done(what);
    assert_eq!((s["phase"].clone(), rows(&s)), (json!("idle"), pairs(&[("webkit", "installed"), ("other", "installed")])), "{what}");
    assert!(scn.store().read(store::JOB).unwrap()["finishedAt"].is_u64(), "{what}: the new owner ended the job");
    let calls = verify(
        &fake,
        mark,
        &Expect { pk: seq(&[check_calls(), install_calls(), one("GetTransactionList"), one("Properties.GetAll"), tx("GetUpdates")]), nm: 6, notify: 1, stubs: 1 },
        what,
    );
    assert_notified_by(&calls, &o, &m, what);
}

/// With a fixture procfs that shows Ubuntu's unpacked release upgrader, every write of design §3.7's list is
/// `update.guard.upgrade` and reaches nothing (`CallCount()` unchanged), while our own running download keeps
/// showing and can still be cancelled (the Cancel reaches the fake; review F17); afterwards the phase is
/// `releaseRunning`. In nl and en.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn upgrade_running_refuses_every_write() {
    let fake = Fake::new();
    for lang in Lang::ALL {
        let what = format!("upgrade running ({})", lang.code());
        let scn = Scenario::new();
        let mark = fake.stub_log().len();
        fake.load("cancel-download");
        fake.run();
        let h = Host::owner(&scn, "a", lang);
        h.check();
        let job = h.install(json!({}));
        h.wait("the cancellable download", LONG, |s| s["job"]["status"] == 8 && s["job"]["allowCancel"] == true);
        write(&fake.fixture.join("proc/4321/cmdline"), UPGRADER);
        let before = fake.call_count();
        for (method, args) in [
            ("update.check", json!({})),
            ("update.install", json!({})),
            ("update.install", json!({"rows":["webkit"]})),
            ("update.confirm", json!({"jobId":job,"accept":true})),
            ("update.repair", json!({})),
            ("update.release.check", json!({})),
            ("update.release.compat", json!({"refresh":true})),
            ("update.release.setPrompt", json!({"prompt":"lts"})),
            ("update.release.start", json!({})),
        ] {
            assert_error(&h.call(method, args).unwrap_err(), "update_busy", "update.guard.upgrade", lang, &format!("{what} {method}"));
        }
        assert_eq!(fake.call_count(), before, "{what}: a refused write reached the fake");
        let s = h.status();
        assert_eq!((s["phase"].clone(), s["release"]["state"].clone()), (json!("downloading"), json!("running")), "{what}: our own job keeps showing");
        // not on §3.7's list: they are not refused by the upgrader guard (they answer for themselves)
        for (method, args) in [
            ("update.confirm", json!({"jobId":job,"accept":false})),
            ("update.setAutoOpen", json!({"on":false})),
            ("update.release.ubuntuHold", json!({"on":true})),
            ("update.release.attach", json!({})),
        ] {
            if let Err(e) = h.call(method, args) {
                assert_ne!(e.key.as_deref(), Some("update.guard.upgrade"), "{what}: {method}");
            }
        }
        let id = h.call("update.status", json!({"packages":true})).unwrap()["packages"][0]["id"].clone();
        assert!(h.call("update.details", json!({"ids":[id]})).is_ok(), "{what}: details");
        assert_eq!(h.call("update.cancel", json!({"jobId":job})).expect("cancel"), json!({"ok":true}));
        let s = h.done(&what);
        assert_eq!(s["phase"], "releaseRunning", "{what}");
        assert_eq!(s["warnings"][0]["key"], "update.err.cancelled");
        fs::remove_dir_all(fake.fixture.join("proc/4321")).unwrap();
        assert_eq!(h.status()["phase"], "available", "{what}: the upgrader is gone");
        verify(
            &fake,
            mark,
            &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetUpdateDetail"), one("Cancel"), tx("GetUpdates"), tx("GetDetails")]), nm: 4, notify: 0, stubs: 2 },
            &what,
        );
    }
}

/// Every update method valid for a dry-run test.
fn all_calls() -> Vec<(&'static str, Value)> {
    let id = "0123456789abcdef0123456789abcdef";
    vec![
        ("update.status", json!({})),
        ("update.status", json!({"packages":true})),
        ("update.check", json!({})),
        ("update.details", json!({"ids":["libsoup-3.0-0;3.6.6-1ubuntu0.3;amd64;ubuntu-resolute-security"]})),
        ("update.install", json!({})),
        ("update.install", json!({"rows":["security"]})),
        ("update.confirm", json!({"jobId":id,"accept":true})),
        ("update.cancel", json!({"jobId":id})),
        ("update.repair", json!({})),
        ("update.history", json!({"limit":10})),
        ("update.options", json!({})),
        ("update.setAutoOpen", json!({"on":false})),
        ("update.release.check", json!({})),
        ("update.release.compat", json!({"refresh":true})),
        ("update.release.setPrompt", json!({"prompt":"lts"})),
        ("update.release.ubuntuHold", json!({"on":true})),
        ("update.release.start", json!({})),
        ("update.release.attach", json!({})),
    ]
}

/// A dry Settings host opens the page and calls every method (check, install, cancel, release.*, …): the
/// dry-run envelopes, no connection to the fake's bus, `CallCount()` 0, no stub call.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn dry_run_makes_no_call_on_the_fake() {
    let fake = Fake::new();
    let scn = Scenario::new();
    let mark = fake.stub_log().len();
    fake.load("available-mixed");
    fake.run();
    let names = fake.names();
    let h = Host::new(&scn, "d", "settings", Lang::Nl, true, None);
    assert_eq!(h.service.mode(), Mode::Dry);
    for (method, args) in all_calls() {
        let result = h.call(method, args);
        match method {
            "update.history" | "update.options" => {} // read-only stubs until GW4
            "update.install" => assert_eq!(result.unwrap(), json!({"jobId":null,"dryRun":true})),
            "update.details" => assert_eq!(result.unwrap(), json!({"details":[],"dryRun":true})),
            "update.release.compat" => assert_eq!(result.unwrap(), json!({"state":"unknown","dryRun":true})),
            _ => assert_eq!(result.unwrap()["dryRun"], true, "{method}"),
        }
    }
    assert_eq!(h.service.stats(), Some((0, 0, 0, 0)), "a bus, a thread, a program or a forward");
    assert_eq!(fake.names(), names, "the dry host connected to the bus");
    verify(&fake, mark, &Expect::default(), "dry run");
}

/// `panel`, `start`, `desktop`, `taskmgr` and `explorer` hosts never connect to the fake: `update.status` reads
/// files, writes are `not_here`, and the power guard there reads files only (it refuses with a committing job of
/// a live owner, still without a call).
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn reader_mode_never_connects() {
    let fake = Fake::new();
    let mark = fake.stub_log().len();
    fake.load("available-mixed");
    fake.run();
    let names = fake.names();
    for kind in ["panel", "start", "desktop", "taskmgr", "explorer"] {
        let scn = Scenario::new();
        let h = Host::new(&scn, "r", kind, Lang::En, false, None);
        assert_eq!(h.service.mode(), Mode::Reader, "{kind}");
        for (method, args) in all_calls() {
            let result = h.call(method, args);
            match method {
                "update.status" => assert_eq!(result.unwrap()["mode"], "reader"),
                "update.history" | "update.options" => {}
                _ => assert_error(&result.unwrap_err(), "not_here", "update.err.notHere", Lang::En, &format!("{kind} {method}")),
            }
        }
        assert!(guard::power_blocked(&h.ctx, "reboot").is_none(), "{kind}: no job");
        scn.store()
            .write(
                store::JOB,
                &json!({"version":1,"jobId":"0123456789abcdef0123456789abcdef","bootId":BOOT_ID,"role":"update","committing":true,"owner":store::current_owner()}),
            )
            .unwrap();
        let refused = guard::power_blocked(&h.ctx, "reboot").expect("refused while committing");
        assert_eq!(refused.key.as_deref(), Some("update.guard.power"));
        assert!(guard::power_blocked(&h.ctx, "lock").is_none());
        assert_eq!(h.service.stats(), Some((0, 0, 0, 0)), "{kind}");
        assert_eq!(fake.names(), names, "{kind} connected to the bus");
    }
    verify(&fake, mark, &Expect::default(), "reader mode");
}

/// `shutdown()` returns in < 100 ms while a fake transaction runs; the next owner reattaches to it
/// (GetTransactionList and GetAll with NO_AUTO_START, `job.reattached`, the transaction's own AllowCancel) and can
/// cancel it, because PackageKit compares uids.
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn shutdown_is_fast_and_the_next_owner_reattaches() {
    let fake = Fake::new();
    let what = "shutdown and reattach";
    let scn = Scenario::new();
    let mark = fake.stub_log().len();
    fake.load("cancel-download");
    fake.run();
    let a = Host::owner(&scn, "a", Lang::Nl);
    a.check();
    let job = a.install(json!({}));
    a.wait("the cancellable download", LONG, |s| s["job"]["status"] == 8 && s["job"]["allowCancel"] == true);
    let took = a.shutdown();
    eprintln!("{what}: shutdown during the transaction took {took:?}");
    assert!(took < SHUTDOWN_LIMIT, "{what}: shutdown took {took:?} while a transaction ran");
    wait_for("the caller's vanish", LONG, || fake.transactions().iter().any(|t| t["vanished"] == true));
    let b = Host::owner(&scn, "b", Lang::Nl);
    let s = b.wait("the reattached job", LONG, |s| s["job"]["reattached"] == true);
    assert_eq!((s["phase"].clone(), s["job"]["jobId"].clone(), s["job"]["allowCancel"].clone()), (json!("downloading"), json!(job), json!(true)), "{what}");
    assert_eq!(b.call("update.cancel", json!({"jobId":job})).expect("cancel after reattach"), json!({"ok":true}));
    let s = b.done(what);
    assert_eq!((s["phase"].clone(), s["warnings"][0]["key"].clone()), (json!("available"), json!("update.err.cancelled")), "{what}");
    let took = b.shutdown();
    eprintln!("{what}: an idle shutdown took {took:?}");
    assert!(took < SHUTDOWN_LIMIT, "{what}: an idle shutdown took {took:?}");
    verify(
        &fake,
        mark,
        &Expect {
            pk: seq(&[check_calls(), install_calls(), one("GetTransactionList"), one("Properties.GetAll"), one("Cancel"), tx("GetUpdates"), tx("GetDetails")]),
            nm: 6,
            notify: 0,
            stubs: 2,
        },
        what,
    );
}

/// packagekitd vanishes mid-download (`SetDaemon(false)`: the name and every transaction are gone without
/// Finished): the outcome is unknown, resolved through GetOldTransactions, whose CreateTransaction D-Bus-activates
/// a new daemon; it has no record of the job, so `update.err.unknownOutcome` (and no re-check). The record case
/// is `finished-in-gap` in [`every_timeline_in_nl_and_en`].
#[test]
#[ignore = "GW2b bus test: only tests/update/run.sh inside the bwrap layer"]
fn a_vanished_daemon_is_resolved_through_old_transactions() {
    let fake = Fake::new();
    for lang in Lang::ALL {
        let what = format!("daemon vanishes ({})", lang.code());
        let scn = Scenario::new();
        let mark = fake.stub_log().len();
        fake.load("cancel-download");
        fake.run();
        let h = Host::owner(&scn, "a", lang);
        h.check();
        h.install(json!({}));
        h.wait("the cancellable download", LONG, |s| s["job"]["status"] == 8 && s["job"]["allowCancel"] == true);
        fake.set_daemon(false);
        let s = h.done(&what);
        assert_eq!(s["phase"], "error", "{what}");
        assert_snapshot_error(&s, "update.err.unknownOutcome", lang, &what);
        assert_eq!(rows(&s), pairs(&[("webkit", "pendingDownload"), ("other", "pendingDownload")]), "{what}: unknown is not failed");
        let state = fake.state();
        assert_eq!((state["activations"].clone(), state["daemon"].clone()), (json!(1), json!(true)), "{what}");
        assert_eq!(scn.store().read(store::JOB).unwrap()["outcome"]["code"], "unknown_outcome");
        verify(&fake, mark, &Expect { pk: seq(&[check_calls(), install_calls(), tx("GetOldTransactions")]), nm: 4, notify: 1, stubs: 1 }, &what);
    }
}
