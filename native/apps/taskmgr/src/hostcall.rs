//! Host calls that can never hang Task Manager, and the host's health (js/apps/taskmgr/hostcall.js).
//!
//! Robin: "Taakbeheer mag nooit kunnen vastlopen, anders kan ik Win11 Shell niet eens opnieuw starten." Every call
//! of Task Manager goes through `call` / `call_with`: it runs off the main thread (kit `Ctx::call`) and has a
//! timeout per method (`timeout_of`); a late answer after the timeout is dropped. The clock starts once the
//! backend is up (while it starts, the kit holds calls back).
//!
//! Health: a call that fails at the transport level (timeout, no connection, garbled or refused answer) is a
//! strike; any answer of the host, also an error, resets the strikes. Two strikes in a row (a failed poll plus the
//! probe right after it), or three failures in a row of one recurring poll (`source`, e.g. "processes": the host
//! answers but sysmon hangs) mean "down". While anything failed recently a light probe (shell.info) runs every
//! few seconds, so Task Manager also notices when the host is back; while all is well the same probe is a
//! heartbeat every HEARTBEAT_MS (window visible only). Its mainLoopLagMs above LOOP_STUCK_MS counts as down too.
//! The WebKit app could also ask its app window to restart the shell when the host was stuck; the native app has
//! no such window, so a restart always goes through the backend (sysmon.restartProcess / shell.restart).
use kit::CallError;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// The default timeout per method (ms); 0 = none.
pub fn timeout_of(method: &str) -> u64 {
    match method {
        "sysmon.processes" | "sysmon.performance" => 4000,
        "sysmon.health" => 10000,
        "sysmon.startup" | "sysmon.services" | "storage.volumes" => 8000,
        "shell.info" => 2000,
        "sysmon.endTask" => 10000,
        "sysmon.setEfficiency" | "sysmon.setStartup" => 8000,
        "sysmon.restartProcess" => 45000,
        "sysmon.serviceAction" => 30000,
        "storage.scan" | "storage.scanResult" => 30000,
        "storage.scanCancel" => 5000,
        "storage.largeFiles" | "storage.categories" => 60000,
        "storage.cleanup.list" => 30000,
        "storage.cleanup.run" => 180000,
        "files.trash" => 60000,
        _ => DEFAULT_TIMEOUT,
    }
}

pub const DEFAULT_TIMEOUT: u64 = 8000;
/// Calls slower than this say little about the host's health when they time out (scans, clean-up).
const HEALTH_MAX_TIMEOUT: u64 = 10000;

/// Errors that mean "the host did not answer" (anything else is an answer).
pub fn is_transport(e: &CallError) -> bool {
    matches!(e.code.as_str(), "timeout" | "offline" | "bad_response" | "unauthorized")
}

fn timeout_error() -> CallError {
    CallError::new("timeout", kit::t("Win11 Shell reageert niet. Probeer het zo opnieuw.", "Win11 Shell is not responding. Try again in a moment."))
}

/// Per-call options (hostcall.js `{timeout, source, health}`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    /// Milliseconds (None: `timeout_of(method)`; Some(0): none).
    pub timeout: Option<u64>,
    /// The recurring poll this call belongs to (a store channel name).
    pub source: Option<&'static str>,
    /// Whether the outcome counts for the health (None: calls with a timeout of at most 10 s).
    pub health: Option<bool>,
}

type Done = Box<dyn FnOnce(Result<Value, CallError>)>;

thread_local! {
    static READY_WAITERS: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
    static READY_HOOKED: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` once the backend is up (now, when it is).
fn when_ready(f: impl FnOnce() + 'static) {
    let Some(ctx) = kit::try_ctx() else { return };
    if ctx.backend().is_ready() {
        f();
        return;
    }
    READY_WAITERS.with(|w| w.borrow_mut().push(Box::new(f)));
    if !READY_HOOKED.with(|h| h.replace(true)) {
        ctx.on("backend.ready", |_, _| {
            let waiters: Vec<Box<dyn FnOnce()>> = READY_WAITERS.with(|w| w.borrow_mut().drain(..).collect());
            for f in waiters {
                f();
            }
        });
    }
}

/// A backend call with the method's timeout; `done` on the main thread, exactly once.
pub fn call(method: &str, args: Value, done: impl FnOnce(Result<Value, CallError>) + 'static) {
    call_with(method, args, Opts::default(), done);
}

/// `call` with options.
pub fn call_with(method: &str, args: Value, opts: Opts, done: impl FnOnce(Result<Value, CallError>) + 'static) {
    let Some(ctx) = kit::try_ctx() else {
        done(Err(CallError::new("offline", kit::t("Geen verbinding met Win11 Shell. Probeer het zo opnieuw.", "No connection to Win11 Shell. Try again in a moment."))));
        return;
    };
    let timeout = opts.timeout.unwrap_or_else(|| timeout_of(method));
    let tracked = opts.health.unwrap_or(timeout > 0 && timeout <= HEALTH_MAX_TIMEOUT);
    let source = opts.source;
    let slot: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = {
        let slot = slot.clone();
        move |r: Result<Value, CallError>| {
            let Some(done) = slot.borrow_mut().take() else { return };
            let h = health();
            match &r {
                Ok(_) => {
                    if tracked {
                        h.sample(true, source);
                    }
                }
                Err(e) if is_transport(e) => {
                    if tracked {
                        h.sample(false, source);
                    } else {
                        h.suspect();
                    }
                }
                Err(_) => {
                    if tracked {
                        h.sample(true, source);
                    }
                }
            }
            done(r);
        }
    };
    let finish = Rc::new(RefCell::new(Some(finish)));
    let f1 = finish.clone();
    ctx.call(method, args, move |r| {
        if let Some(f) = f1.borrow_mut().take() {
            f(r);
        }
    });
    if timeout > 0 {
        let f2 = finish.clone();
        when_ready(move || {
            glib::timeout_add_local_once(Duration::from_millis(timeout), move || {
                if let Some(f) = f2.borrow_mut().take() {
                    f(Err(timeout_error()));
                }
            });
        });
    }
}

// ------------------------------------------------------------------------------------------------ health

const PROBE_MS: u64 = 2500;
const PROBE_FAST_MS: u64 = 1200;
const HEARTBEAT_MS: f64 = 5000.0;
const STREAK_TTL: f64 = 15000.0;
const RESTART_MIN_MS: f64 = 1500.0;
const RESTART_QUIET_MS: f64 = 12000.0;
const RESTART_GIVEUP_MS: f64 = 30000.0;
const LOOP_STUCK_MS: f64 = 8000.0;

/// The host's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ok,
    Down,
    Restarting,
}

/// What came with a state change (hostcall.js `info`).
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub last_ok_at: f64,
    pub since: f64,
    pub restarting: bool,
    pub prev: Option<State>,
    /// Back after a restart we asked for.
    pub restarted: bool,
    /// The restart did not bring it back within RESTART_GIVEUP_MS.
    pub gave_up: bool,
    /// A restart we started was cancelled (refused, test mode).
    pub cancelled: bool,
}

struct Restart {
    at: f64,
    saw_fail: bool,
    via_api: bool,
}

type Listener = Rc<dyn Fn(State, &Info)>;

/// The host's health (one per process: `health()`).
pub struct Health {
    state: Cell<State>,
    since: Cell<f64>,
    last_ok_at: Cell<f64>,
    strikes: Cell<u32>,
    streaks: RefCell<HashMap<&'static str, (u32, f64)>>,
    subs: RefCell<Vec<(u64, Listener)>>,
    next_id: Cell<u64>,
    probe_timer: RefCell<Option<glib::SourceId>>,
    probing: Cell<bool>,
    last_probe_at: Cell<f64>,
    loop_stuck: Cell<bool>,
    run_id: RefCell<Option<String>>,
    restart: RefCell<Option<Restart>>,
    visible: Cell<bool>,
}

thread_local! {
    static HEALTH: RefCell<Option<Rc<Health>>> = const { RefCell::new(None) };
}

/// The health tracker (made on first use; it starts the heartbeat).
pub fn health() -> Rc<Health> {
    if let Some(h) = HEALTH.with(|h| h.borrow().clone()) {
        return h;
    }
    let h = Rc::new(Health {
        state: Cell::new(State::Ok),
        since: Cell::new(now()),
        last_ok_at: Cell::new(0.0),
        strikes: Cell::new(0),
        streaks: RefCell::new(HashMap::new()),
        subs: RefCell::new(Vec::new()),
        next_id: Cell::new(1),
        probe_timer: RefCell::new(None),
        probing: Cell::new(false),
        last_probe_at: Cell::new(0.0),
        loop_stuck: Cell::new(false),
        run_id: RefCell::new(None),
        restart: RefCell::new(None),
        visible: Cell::new(true),
    });
    HEALTH.with(|slot| *slot.borrow_mut() = Some(h.clone()));
    if let Some(ctx) = kit::try_ctx() {
        // the event stream drops / comes back when the host goes away / is back
        ctx.on("bridge.status", |_, d| {
            let h = health();
            let connected = d["connected"].as_bool().unwrap_or(false);
            if !connected && let Some(r) = h.restart.borrow_mut().as_mut() {
                r.saw_fail = true;
            }
            h.schedule(Some(if connected { 50 } else { 300 }));
        });
    }
    h.schedule(None);
    h
}

fn now() -> f64 {
    kit::format::now_ms()
}

impl Health {
    pub fn state(&self) -> State {
        self.state.get()
    }

    pub fn is_ok(&self) -> bool {
        self.state.get() == State::Ok
    }

    /// When the host last answered (ms; 0: never).
    pub fn last_ok_at(&self) -> f64 {
        self.last_ok_at.get()
    }

    /// Run `f(state, info)` on every change; the id removes it again (`off`).
    pub fn on_change(&self, f: impl Fn(State, &Info) + 'static) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.subs.borrow_mut().push((id, Rc::new(f)));
        id
    }

    pub fn off(&self, id: u64) {
        self.subs.borrow_mut().retain(|(i, _)| *i != id);
    }

    /// The window was hidden or shown (no heartbeat while hidden).
    pub fn set_visible(&self, visible: bool) {
        if self.visible.replace(visible) == visible {
            return;
        }
        if visible {
            self.schedule(None);
        }
    }

    fn set_state(&self, s: State, mut extra: Info) {
        if s == self.state.get() {
            return;
        }
        let prev = self.state.replace(s);
        self.since.set(now());
        if s == State::Down {
            eprintln!("[taskmgr] Win11 Shell does not answer");
        } else if s == State::Ok && prev != State::Ok {
            eprintln!("[taskmgr] Win11 Shell answers again");
        }
        extra.last_ok_at = self.last_ok_at.get();
        extra.since = self.since.get();
        extra.restarting = self.restart.borrow().is_some();
        extra.prev = Some(prev);
        let subs: Vec<Listener> = self.subs.borrow().iter().map(|(_, f)| f.clone()).collect();
        for f in subs {
            f(s, &extra);
        }
    }

    fn streak_down(&self) -> bool {
        let t = now();
        let mut down = false;
        self.streaks.borrow_mut().retain(|_, (n, at)| {
            if t - *at > STREAK_TTL {
                return false;
            }
            if *n >= 3 {
                down = true;
            }
            true
        });
        down
    }

    fn failing(&self) -> bool {
        self.strikes.get() > 0 || !self.streaks.borrow().is_empty()
    }

    fn evaluate(&self) {
        let down = self.strikes.get() >= 2 || self.streak_down() || self.loop_stuck.get();
        let restart = self.restart.borrow().as_ref().map(|r| (r.at, r.saw_fail, r.via_api));
        match restart {
            Some((at, saw_fail, _via_api)) => {
                let age = now() - at;
                if !down && !self.failing() && age >= RESTART_MIN_MS && (saw_fail || age >= RESTART_QUIET_MS) {
                    *self.restart.borrow_mut() = None;
                    self.set_state(State::Ok, Info { restarted: saw_fail, ..Default::default() });
                } else if down && age >= RESTART_GIVEUP_MS {
                    *self.restart.borrow_mut() = None;
                    self.set_state(State::Down, Info { gave_up: true, ..Default::default() });
                }
            }
            None => self.set_state(if down { State::Down } else { State::Ok }, Info::default()),
        }
        self.schedule(None);
    }

    fn schedule(&self, ms: Option<u64>) {
        if let Some(id) = self.probe_timer.borrow_mut().take() {
            id.remove();
        }
        if self.probing.get() || !self.visible.get() {
            return;
        }
        let ms = ms.unwrap_or_else(|| {
            if !self.failing() && self.state.get() == State::Ok && !self.loop_stuck.get() {
                (self.last_probe_at.get() + HEARTBEAT_MS - now()).max(1000.0) as u64
            } else if self.restart.borrow().is_some() {
                PROBE_FAST_MS
            } else {
                PROBE_MS
            }
        });
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), || {
            let h = health();
            h.probe_timer.borrow_mut().take();
            h.probe();
        });
        *self.probe_timer.borrow_mut() = Some(id);
    }

    fn probe(&self) {
        if self.probing.replace(true) {
            return;
        }
        self.last_probe_at.set(now());
        call_with("shell.info", json!({}), Opts { timeout: Some(timeout_of("shell.info")), source: None, health: Some(true) }, |r| {
            let h = health();
            if let Ok(info) = r {
                let lag = info["mainLoopLagMs"].as_f64().unwrap_or(0.0);
                let stuck = lag > LOOP_STUCK_MS;
                if stuck != h.loop_stuck.get() {
                    h.loop_stuck.set(stuck);
                    if stuck {
                        eprintln!("[taskmgr] Win11 Shell answers, but its main loop has not run for {} s", (lag / 1000.0).round());
                    }
                }
                if let Some(r) = h.restart.borrow_mut().as_mut() {
                    if stuck {
                        r.saw_fail = true;
                    }
                    let id = info["runId"].as_str();
                    if let (Some(id), Some(old)) = (id, h.run_id.borrow().as_deref())
                        && id != old
                    {
                        r.saw_fail = true;
                    }
                }
                if let Some(id) = info["runId"].as_str() {
                    *h.run_id.borrow_mut() = Some(id.to_owned());
                }
            }
            h.probing.set(false);
            h.evaluate();
            if h.strikes.get() == 1 {
                h.schedule(Some(300));
            }
        });
    }

    /// One call's outcome (see the module docs).
    pub fn sample(&self, ok: bool, source: Option<&'static str>) {
        if ok {
            self.last_ok_at.set(now());
            self.strikes.set(0);
            if let Some(s) = source {
                self.streaks.borrow_mut().remove(s);
            }
        } else {
            self.strikes.set(self.strikes.get() + 1);
            if let Some(r) = self.restart.borrow_mut().as_mut() {
                r.saw_fail = true;
            }
            if let Some(s) = source {
                let mut st = self.streaks.borrow_mut();
                let n = st.get(s).map(|(n, _)| *n).unwrap_or(0);
                st.insert(s, (n + 1, now()));
            }
        }
        self.evaluate();
        if !ok && self.strikes.get() == 1 {
            self.schedule(Some(0));
        }
    }

    /// Something slow failed: check the host without counting it.
    pub fn suspect(&self) {
        self.schedule(Some(0));
    }

    /// The host said it restarts itself (sysmon.restartProcess -> how:"shell").
    pub fn expect_restart(&self, via_api: bool) {
        *self.restart.borrow_mut() = Some(Restart { at: now(), saw_fail: false, via_api });
        self.set_state(State::Restarting, Info::default());
        self.schedule(None);
    }

    fn cancel_restart(&self) {
        *self.restart.borrow_mut() = None;
        let down = self.strikes.get() >= 2 || self.streak_down();
        self.set_state(if down { State::Down } else { State::Ok }, Info { cancelled: true, ..Default::default() });
        self.schedule(None);
    }

    /// Restart Win11 Shell; `pid` is the shell's pid (its row in Processes), when known. `done(Ok((how, result)))`
    /// with how "api", "dryRun" or "none" (no answer: the banner keeps waiting), or the host's refusal.
    pub fn restart_shell(self: &Rc<Self>, pid: Option<i64>, done: impl FnOnce(Result<(&'static str, Value), CallError>) + 'static) {
        let (method, args) = match pid {
            Some(pid) => ("sysmon.restartProcess", json!({ "pid": pid })),
            None => ("shell.restart", json!({})),
        };
        let healthy_with_pid = self.is_ok() && pid.is_some();
        self.expect_restart(false);
        let me = self.clone();
        call_with(method, args, Opts { timeout: Some(6000), source: None, health: Some(false) }, move |r| match r {
            Ok(res) => {
                if res["dryRun"] == true {
                    me.cancel_restart();
                    done(Ok(("dryRun", res)));
                } else if res["ok"] == false {
                    me.cancel_restart();
                    let msg = res["error"].as_str().unwrap_or(kit::t("Er ging iets mis.", "Something went wrong.")).to_owned();
                    done(Err(CallError::new("failed", msg)));
                } else {
                    if let Some(r) = me.restart.borrow_mut().as_mut() {
                        r.via_api = true;
                    }
                    done(Ok(("api", res)));
                }
            }
            Err(e) => {
                if !healthy_with_pid && is_transport(&e) {
                    // no answer: the host may be on its way down already; the banner waits for it
                    done(Ok(("none", Value::Null)));
                } else {
                    me.cancel_restart();
                    done(Err(e));
                }
            }
        });
    }
}
