//! Task Manager's data store (js/apps/taskmgr/store.js): it polls sysmon / storage only while a page needs the
//! data, at the chosen update speed (taskmgr.updateSpeed), pauses while the window is hidden and keeps the
//! performance history (for the graphs) in memory. Every poll has a timeout (hostcall.rs); while Win11 Shell does
//! not answer the last data stays and the realtime polls slow down; when it is back, every open channel
//! refreshes.
//!
//! ```ignore
//! let sub = app.store.subscribe("performance", |data| { ... });   // starts polling; dropping `sub` stops it
//! app.store.get("processes");  app.store.refresh("processes");  app.store.history()
//! app.store.on_speed(|ms, key| { ... });
//! ```
//!
//! Channels: "processes" (sysmon.processes, realtime), "performance" (sysmon.performance, realtime), "health"
//! (sysmon.health, every 30 s), "volumes" (storage.volumes, every 60 s), "startup" (sysmon.startup, once),
//! "services" (sysmon.services, every 10 s).
use crate::hostcall::{self, Opts, State};
use kit::CallError;
use kit::prelude::*;
use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// taskmgr.updateSpeed -> poll interval in ms (0: paused).
pub fn speed_of(key: &str) -> Option<u64> {
    Some(match key {
        "high" => 500,
        "normal" => 1000,
        "low" => 4000,
        "paused" => 0,
        _ => return None,
    })
}

/// Realtime polls while the host does not answer.
const DOWN_INTERVAL: u64 = 2500;
/// Samples per history series (10 min at 0.5 s).
pub const MAX_HISTORY: usize = 1200;

struct Def {
    name: &'static str,
    method: &'static str,
    realtime: bool,
    /// ms between polls of a non-realtime channel (0: once).
    every: u64,
}

static CHANNELS: [Def; 6] = [
    Def { name: "processes", method: "sysmon.processes", realtime: true, every: 0 },
    Def { name: "performance", method: "sysmon.performance", realtime: true, every: 0 },
    Def { name: "health", method: "sysmon.health", realtime: false, every: 30000 },
    Def { name: "volumes", method: "storage.volumes", realtime: false, every: 60000 },
    Def { name: "startup", method: "sysmon.startup", realtime: false, every: 0 },
    Def { name: "services", method: "sysmon.services", realtime: false, every: 10000 },
];

type Sub = Rc<dyn Fn(&Rc<Value>)>;
type ErrSub = Rc<dyn Fn(&CallError)>;
type Waiter = Box<dyn FnOnce(Option<Rc<Value>>)>;

struct Channel {
    def: &'static Def,
    subs: Vec<(u64, Sub)>,
    err_subs: Vec<(u64, ErrSub)>,
    timer: Option<glib::SourceId>,
    data: Option<Rc<Value>>,
    error: Option<CallError>,
    inflight: bool,
    inflight_lang: &'static str,
    seq: u64,
    last_at: f64,
    last_try: f64,
    waiters: Vec<(u64, Waiter)>,
}

/// One disk's history: active time %, read and write bytes per second.
#[derive(Clone, Debug, Default)]
pub struct DiskHistory {
    pub active: VecDeque<f64>,
    pub read: VecDeque<f64>,
    pub write: VecDeque<f64>,
}

/// One network interface's history: received and sent bytes per second.
#[derive(Clone, Debug, Default)]
pub struct NetHistory {
    pub rx: VecDeque<f64>,
    pub tx: VecDeque<f64>,
}

/// One GPU's history: usage % and memory %.
#[derive(Clone, Debug, Default)]
pub struct GpuHistory {
    pub usage: VecDeque<f64>,
    pub mem: VecDeque<f64>,
}

/// The performance history (store.js `history`), at most MAX_HISTORY samples per series, oldest first.
#[derive(Clone, Debug, Default)]
pub struct History {
    /// Sample times (ms).
    pub t: VecDeque<f64>,
    /// CPU usage %.
    pub cpu: VecDeque<f64>,
    /// Usage % per logical processor.
    pub cpu_cores: Vec<VecDeque<f64>>,
    /// Average frequency in MHz.
    pub freq: VecDeque<f64>,
    /// Memory in use, %.
    pub mem: VecDeque<f64>,
    /// Memory in use, bytes.
    pub mem_used: VecDeque<f64>,
    /// By disk name (performance.disks[].name).
    pub disks: BTreeMap<String, DiskHistory>,
    /// By interface (performance.network[].iface).
    pub nets: BTreeMap<String, NetHistory>,
    /// By GPU index.
    pub gpus: BTreeMap<usize, GpuHistory>,
    /// Samples recorded so far.
    pub count: u64,
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// The last `n` values of a series (fewer when it is shorter), oldest first.
pub fn tail(series: &VecDeque<f64>, n: usize) -> Vec<f64> {
    let skip = series.len().saturating_sub(n);
    series.iter().skip(skip).copied().collect()
}

fn push(series: &mut VecDeque<f64>, v: f64) {
    series.push_back(if v.is_finite() { v } else { 0.0 });
    while series.len() > MAX_HISTORY {
        series.pop_front();
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

impl History {
    fn record(&mut self, p: &Value) {
        if !p["cpu"].is_object() {
            return;
        }
        push(&mut self.t, p["timestamp"].as_f64().unwrap_or_else(kit::format::now_ms));
        push(&mut self.cpu, f(&p["cpu"]["usage"]));
        push(&mut self.freq, f(&p["cpu"]["freqMHz"]));
        if let Some(cores) = p["cpu"]["perCore"].as_array() {
            for (i, v) in cores.iter().enumerate() {
                if self.cpu_cores.len() <= i {
                    self.cpu_cores.resize_with(i + 1, VecDeque::new);
                }
                push(&mut self.cpu_cores[i], f(v));
            }
        }
        let m = &p["memory"];
        let total = f(&m["total"]);
        push(&mut self.mem_used, f(&m["used"]));
        push(&mut self.mem, if total > 0.0 { f(&m["used"]) / total * 100.0 } else { 0.0 });
        for d in p["disks"].as_array().into_iter().flatten() {
            let name = kit::table::value_text(&d["name"]);
            let hd = self.disks.entry(name).or_default();
            push(&mut hd.active, f(&d["activePct"]));
            push(&mut hd.read, f(&d["readBps"]));
            push(&mut hd.write, f(&d["writeBps"]));
        }
        for n in p["network"].as_array().into_iter().flatten() {
            let name = kit::table::value_text(&n["iface"]);
            let hn = self.nets.entry(name).or_default();
            push(&mut hn.rx, f(&n["rxBps"]));
            push(&mut hn.tx, f(&n["txBps"]));
        }
        for (i, g) in p["gpus"].as_array().into_iter().flatten().enumerate() {
            let hg = self.gpus.entry(i).or_default();
            push(&mut hg.usage, f(&g["usage"]));
            let total = f(&g["memTotal"]);
            push(&mut hg.mem, if total > 0.0 { f(&g["memUsed"]) / total * 100.0 } else { 0.0 });
        }
        self.count += 1;
    }
}

struct Inner {
    channels: RefCell<Vec<Channel>>,
    history: RefCell<History>,
    speed_key: RefCell<String>,
    speed_subs: RefCell<Vec<(u64, Rc<dyn Fn(u64, &str)>)>>,
    next_id: Cell<u64>,
    visible: Cell<bool>,
}

/// The store (a cheap handle).
#[derive(Clone)]
pub struct Store {
    inner: Rc<Inner>,
}

/// A subscription; dropping it unsubscribes (the last subscriber stops the polling).
pub struct Subscription {
    store: Weak<Inner>,
    channel: usize,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let Some(inner) = self.store.upgrade() else { return };
        let Ok(mut chs) = inner.channels.try_borrow_mut() else {
            // dropped from inside a channel borrow (should not happen): try again soon
            let (store, channel, id) = (self.store.clone(), self.channel, self.id);
            glib::idle_add_local_once(move || drop(Subscription { store, channel, id }));
            return;
        };
        let Some(c) = chs.get_mut(self.channel) else { return };
        c.subs.retain(|(i, _)| *i != self.id);
        c.err_subs.retain(|(i, _)| *i != self.id);
        if c.subs.is_empty()
            && let Some(t) = c.timer.take()
        {
            t.remove();
        }
    }
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// An on_speed listener; dropping it removes it.
pub struct SpeedSub {
    store: Weak<Inner>,
    id: u64,
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
impl Drop for SpeedSub {
    fn drop(&mut self) {
        if let Some(inner) = self.store.upgrade() {
            inner.speed_subs.borrow_mut().retain(|(i, _)| *i != self.id);
        }
    }
}

fn now() -> f64 {
    kit::format::now_ms()
}

impl Store {
    pub fn new() -> Store {
        let ctx = kit::ctx();
        let key = ctx.setting("taskmgr.updateSpeed").as_str().unwrap_or("normal").to_owned();
        let key = if speed_of(&key).is_some() { key } else { "normal".to_owned() };
        let channels = CHANNELS
            .iter()
            .map(|def| Channel {
                def,
                subs: Vec::new(),
                err_subs: Vec::new(),
                timer: None,
                data: None,
                error: None,
                inflight: false,
                inflight_lang: "",
                seq: 0,
                last_at: 0.0,
                last_try: 0.0,
                waiters: Vec::new(),
            })
            .collect();
        let store = Store {
            inner: Rc::new(Inner {
                channels: RefCell::new(channels),
                history: RefCell::new(History::default()),
                speed_key: RefCell::new(key),
                speed_subs: RefCell::new(Vec::new()),
                next_id: Cell::new(1),
                visible: Cell::new(true),
            }),
        };
        // back from "reageert niet": fetch everything that is on screen now
        let weak = Rc::downgrade(&store.inner);
        hostcall::health().on_change(move |state, info| {
            let Some(inner) = weak.upgrade() else { return };
            if state != State::Ok || info.prev == Some(State::Ok) {
                return;
            }
            let me = Store { inner };
            for i in me.active() {
                me.clear_timer(i);
                me.poll_then_schedule(i);
            }
        });
        let weak = Rc::downgrade(&store.inner);
        ctx.on_settings(move |s, _| {
            let Some(inner) = weak.upgrade() else { return };
            let me = Store { inner };
            let k = s["taskmgr"]["updateSpeed"].as_str().unwrap_or("normal");
            if *me.inner.speed_key.borrow() == k || speed_of(k).is_none() {
                return;
            }
            *me.inner.speed_key.borrow_mut() = k.to_owned();
            let realtime: Vec<usize> = me.active().into_iter().filter(|&i| me.inner.channels.borrow()[i].def.realtime).collect();
            for i in realtime {
                me.clear_timer(i);
                if me.speed_ms() > 0 {
                    me.schedule(i);
                }
            }
            let subs: Vec<Rc<dyn Fn(u64, &str)>> = me.inner.speed_subs.borrow().iter().map(|(_, f)| f.clone()).collect();
            let (ms, key) = (me.speed_ms(), me.speed_key());
            for f in subs {
                f(ms, &key);
            }
        });
        store
    }

    fn index(&self, name: &str) -> Option<usize> {
        CHANNELS.iter().position(|d| d.name == name)
    }

    /// Channels with subscribers.
    fn active(&self) -> Vec<usize> {
        self.inner.channels.borrow().iter().enumerate().filter(|(_, c)| !c.subs.is_empty()).map(|(i, _)| i).collect()
    }

    fn next_id(&self) -> u64 {
        let id = self.inner.next_id.get();
        self.inner.next_id.set(id + 1);
        id
    }

    /// The poll interval of the update speed in ms (0: paused).
    pub fn speed_ms(&self) -> u64 {
        speed_of(&self.inner.speed_key.borrow()).unwrap_or(1000)
    }

    /// taskmgr.updateSpeed in effect ("high", "normal", "low", "paused").
    pub fn speed_key(&self) -> String {
        self.inner.speed_key.borrow().clone()
    }

    #[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
    /// The update speed is "paused".
    pub fn paused(&self) -> bool {
        self.speed_ms() == 0
    }

    #[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
    /// Run `f(ms, key)` after the update speed changed.
    pub fn on_speed(&self, f: impl Fn(u64, &str) + 'static) -> SpeedSub {
        let id = self.next_id();
        self.inner.speed_subs.borrow_mut().push((id, Rc::new(f)));
        SpeedSub { store: Rc::downgrade(&self.inner), id }
    }

    #[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
    /// The performance history.
    pub fn history(&self) -> Ref<'_, History> {
        self.inner.history.borrow()
    }

    fn interval_of(&self, i: usize) -> u64 {
        let def = self.inner.channels.borrow()[i].def;
        if def.realtime {
            let ms = self.speed_ms();
            if ms > 0 && !hostcall::health().is_ok() { ms.max(DOWN_INTERVAL) } else { ms }
        } else {
            def.every
        }
    }

    fn clear_timer(&self, i: usize) {
        if let Some(t) = self.inner.channels.borrow_mut()[i].timer.take() {
            t.remove();
        }
    }

    /// Poll a channel now; `then` runs when this poll (or the one already running) is done.
    fn poll(&self, i: usize, then: Option<Waiter>) {
        let lang = kit::lang();
        let (method, name, seq) = {
            let mut chs = self.inner.channels.borrow_mut();
            let c = &mut chs[i];
            if c.inflight && c.inflight_lang == lang {
                if let Some(t) = then {
                    c.waiters.push((c.seq, t));
                }
                return;
            }
            c.seq += 1;
            c.inflight = true;
            c.inflight_lang = lang;
            if let Some(t) = then {
                c.waiters.push((c.seq, t));
            }
            (c.def.method, c.def.name, c.seq)
        };
        let weak = Rc::downgrade(&self.inner);
        hostcall::call_with(method, json!({}), Opts { source: Some(name), ..Default::default() }, move |r| {
            let Some(inner) = weak.upgrade() else { return };
            let me = Store { inner };
            let current = me.inner.channels.borrow()[i].seq == seq;
            let mut deliver: Option<(Vec<Sub>, Rc<Value>)> = None;
            let mut fail: Option<(Vec<ErrSub>, CallError)> = None;
            if current {
                match r {
                    Ok(v) => {
                        let data = Rc::new(v);
                        if name == "performance" {
                            me.inner.history.borrow_mut().record(&data);
                        }
                        let mut chs = me.inner.channels.borrow_mut();
                        let c = &mut chs[i];
                        c.data = Some(data.clone());
                        c.error = None;
                        c.last_at = now();
                        deliver = Some((c.subs.iter().map(|(_, f)| f.clone()).collect(), data));
                    }
                    Err(e) => {
                        eprintln!("[taskmgr] {method} failed: {e}");
                        let mut chs = me.inner.channels.borrow_mut();
                        let c = &mut chs[i];
                        c.error = Some(e.clone());
                        fail = Some((c.err_subs.iter().map(|(_, f)| f.clone()).collect(), e));
                    }
                }
                let mut chs = me.inner.channels.borrow_mut();
                chs[i].inflight = false;
                chs[i].last_try = now();
            }
            if let Some((subs, data)) = deliver {
                for f in subs {
                    f(&data);
                }
            }
            if let Some((subs, e)) = fail {
                for f in subs {
                    f(&e);
                }
            }
            let (waiters, data) = {
                let mut chs = me.inner.channels.borrow_mut();
                let c = &mut chs[i];
                let (mine, rest): (Vec<(u64, Waiter)>, Vec<(u64, Waiter)>) = c.waiters.drain(..).partition(|(g, _)| *g <= seq);
                c.waiters = rest;
                (mine, c.data.clone())
            };
            for (_, w) in waiters {
                w(data.clone());
            }
        });
    }

    fn poll_then_schedule(&self, i: usize) {
        let weak = Rc::downgrade(&self.inner);
        self.poll(
            i,
            Some(Box::new(move |_| {
                if let Some(inner) = weak.upgrade() {
                    Store { inner }.schedule(i);
                }
            })),
        );
    }

    fn schedule(&self, i: usize) {
        self.clear_timer(i);
        let (has_subs, error, last_try, last_at) = {
            let chs = self.inner.channels.borrow();
            let c = &chs[i];
            (!c.subs.is_empty(), c.error.is_some(), c.last_try, c.last_at)
        };
        if !has_subs || !self.inner.visible.get() {
            return;
        }
        let iv = self.interval_of(i);
        if iv == 0 {
            return;
        }
        // after a failure wait a full interval (not "overdue since the last success"): no back-to-back retries
        let base = if error { last_try } else { last_at };
        let due = (base + iv as f64 - now()).max(30.0) as u64;
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local_once(Duration::from_millis(due), move || {
            let Some(inner) = weak.upgrade() else { return };
            let me = Store { inner };
            me.inner.channels.borrow_mut()[i].timer.take();
            me.poll_then_schedule(i);
        });
        self.inner.channels.borrow_mut()[i].timer = Some(id);
    }

    fn start(&self, i: usize) {
        let fresh = {
            let chs = self.inner.channels.borrow();
            let c = &chs[i];
            c.data.is_some() && now() - c.last_at < (self.interval_of(i).max(400)) as f64
        };
        if fresh {
            self.schedule(i);
        } else {
            self.poll_then_schedule(i);
        }
    }

    /// Subscribe to a channel: `f(data)` now (when there is data) and after every poll. The first subscriber
    /// starts the polling. Unknown names never deliver.
    pub fn subscribe(&self, name: &str, f: impl Fn(&Rc<Value>) + 'static) -> Subscription {
        self.subscribe_with_error(name, f, |_| {})
    }

    /// `subscribe` with `on_error(e)` after a failed poll.
    pub fn subscribe_with_error(&self, name: &str, f: impl Fn(&Rc<Value>) + 'static, on_error: impl Fn(&CallError) + 'static) -> Subscription {
        let id = self.next_id();
        let Some(i) = self.index(name) else {
            eprintln!("[taskmgr] unknown store channel {name}");
            return Subscription { store: Weak::new(), channel: 0, id };
        };
        let f: Sub = Rc::new(f);
        let (data, first) = {
            let mut chs = self.inner.channels.borrow_mut();
            let c = &mut chs[i];
            c.subs.push((id, f.clone()));
            c.err_subs.push((id, Rc::new(on_error)));
            (c.data.clone(), c.subs.len() == 1)
        };
        if let Some(d) = data {
            f(&d);
        }
        if first {
            self.start(i);
        }
        Subscription { store: Rc::downgrade(&self.inner), channel: i, id }
    }

    /// The channel's last data.
    pub fn get(&self, name: &str) -> Option<Rc<Value>> {
        let i = self.index(name)?;
        self.inner.channels.borrow()[i].data.clone()
    }

    #[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
    /// The channel's last error (None after a successful poll).
    pub fn error(&self, name: &str) -> Option<CallError> {
        let i = self.index(name)?;
        self.inner.channels.borrow()[i].error.clone()
    }

    /// Data that arrived as an event (storage.volumesChanged).
    pub fn inject(&self, name: &str, data: Value) {
        let Some(i) = self.index(name) else { return };
        if data.is_null() {
            return;
        }
        let data = Rc::new(data);
        let subs: Vec<Sub> = {
            let mut chs = self.inner.channels.borrow_mut();
            let c = &mut chs[i];
            c.data = Some(data.clone());
            c.last_at = now();
            c.subs.iter().map(|(_, f)| f.clone()).collect()
        };
        for f in subs {
            f(&data);
        }
    }

    /// Poll now (F5, after an action).
    pub fn refresh(&self, name: &str) {
        self.refresh_then(name, |_| {});
    }

    /// `refresh`, then `f(new data)` (the last data when the poll failed).
    pub fn refresh_then(&self, name: &str, f: impl FnOnce(Option<Rc<Value>>) + 'static) {
        let Some(i) = self.index(name) else {
            f(None);
            return;
        };
        let weak = Rc::downgrade(&self.inner);
        self.poll(
            i,
            Some(Box::new(move |d| {
                if let Some(inner) = weak.upgrade() {
                    Store { inner }.schedule(i);
                }
                f(d);
            })),
        );
    }

    /// Poll every channel that has subscribers (F5 on a page without its own refresh).
    pub fn refresh_all(&self) {
        for i in self.active() {
            self.poll_then_schedule(i);
        }
    }

    /// Fetch once without subscribing (the cache when it is at most `max_age` ms old).
    pub fn fetch(&self, name: &str, max_age: u64, f: impl FnOnce(Option<Rc<Value>>) + 'static) {
        let Some(i) = self.index(name) else {
            f(None);
            return;
        };
        let cached = {
            let chs = self.inner.channels.borrow();
            let c = &chs[i];
            c.data.clone().filter(|_| now() - c.last_at < max_age as f64)
        };
        match cached {
            Some(d) => f(Some(d)),
            None => self.poll(i, Some(Box::new(f))),
        }
    }

    /// The language changed: cached answers hold the host's texts in the old one. Poll what is on screen now; a
    /// channel subscribed later polls on its first subscriber (nothing counts as fresh any more).
    pub fn lang_changed(&self) {
        let n = self.inner.channels.borrow().len();
        for i in 0..n {
            let has = {
                let mut chs = self.inner.channels.borrow_mut();
                chs[i].last_at = 0.0;
                !chs[i].subs.is_empty()
            };
            if has {
                self.clear_timer(i);
                self.poll_then_schedule(i);
            }
        }
    }

    /// The window was hidden (minimized, suspended) or shown again: no polling while hidden.
    pub fn set_visible(&self, visible: bool) {
        if self.inner.visible.replace(visible) == visible {
            return;
        }
        let n = self.inner.channels.borrow().len();
        for i in 0..n {
            if visible {
                if !self.inner.channels.borrow()[i].subs.is_empty() {
                    self.start(i);
                }
            } else {
                self.clear_timer(i);
            }
        }
    }
}
