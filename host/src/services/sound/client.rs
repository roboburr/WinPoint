//! The PulseAudio client of a host process (amendment 39): one connection per process, on a thread of its own with
//! its own GLib MainContext and libpulse's GLib main loop on that context (libpulse-mainloop-glib, as GNOME
//! Settings' libgvc runs on GTK's). Every libpulse call happens on that thread: callers hand it a job
//! ([`Client::run`], [`Client::write`]) and wait for the answer.
//!
//! On READY it subscribes to sinks, sources, streams, clients, cards and the server, reads all of them and
//! ext-stream-restore's event role, and keeps the [`Model`] current from the subscription events. A change is
//! published 40 ms after the last one (a burst is one [`Event::Changed`]): the Sound service emits `sound.changed`,
//! system.audio's monitor (system/audio_monitor.rs) marks its domain dirty, so both follow one source of truth.
//! A lost server (pipewire-pulse restarted) is reconnected after 1, 2, 5, then every 10 s.
//!
//! Level meters ([`Engine::meter`]) are peak-detect record streams as GNOME Settings' level bars make them (float32,
//! one channel, 25 Hz, PA_STREAM_PEAK_DETECT | DONT_MOVE | ADJUST_LATENCY). They carry GNOME Settings' own
//! application.id, so GNOME Shell's microphone indicator and every libgvc mixer skip them as they skip GNOME
//! Settings' level bars (gnome-shell's volume.js and libgvc filter that id), and so does `sound.state`. A meter
//! lives for a lease the page renews and stops when the page says so.
use super::model::{self, CardPort, Device, EventRole, Model, Port, Profile, Stream};
use super::pulse::{self, Libs};
use glib::translate::ToGlibPtr;
use libc::{c_int, c_void};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ffi::CString,
    ptr,
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// GNOME Settings' application.id on its level bars and test sounds (module docs).
pub(crate) const METER_APP_ID: &str = "org.gnome.VolumeControl";
/// How long a call waits for the sound thread or the server.
const WAIT: Duration = Duration::from_secs(5);
/// A meter's lease (sound.meter {on: true} renews it).
pub(crate) const METER_LEASE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Event {
    /// The model changed (or the server went away).
    Changed,
    /// A meter's peak, 0..1.
    Level {
        kind: String,
        name: String,
        peak: f64,
    },
}

pub(crate) type Listener = Arc<dyn Fn(&Event) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Conn {
    /// the first connection attempt has not ended yet
    Connecting,
    Ready,
    Down,
}

/// Why a call failed (sound.rs renders it in the caller's language).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fail {
    /// no libpulse, no server, the sound thread did not answer
    Unavailable,
    /// the device, card or stream is not there (any more)
    Gone,
    /// an argument the server cannot take (a port the device lacks, a stream without a volume)
    Bad,
    /// the server refused (its own message)
    Failed(String),
}

struct Shared {
    conn: Mutex<Conn>,
    cond: Condvar,
    snapshot: Mutex<Option<Model>>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next: AtomicU64,
}

impl Shared {
    fn set_conn(&self, c: Conn) {
        *self.conn.lock().unwrap_or_else(|e| e.into_inner()) = c;
        self.cond.notify_all();
    }

    fn notify(&self, event: &Event) {
        let listeners: Vec<Listener> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for l in listeners {
            l(event);
        }
    }
}

/// The process's PulseAudio client (module docs).
pub(crate) struct Client {
    context: glib::MainContext,
    shared: Arc<Shared>,
    max_volume: u32,
}

static CLIENT: OnceLock<Option<Arc<Client>>> = OnceLock::new();

/// Where to connect: libpulse's own default (PULSE_SERVER, client.conf, $XDG_RUNTIME_DIR/pulse/native) in a real
/// host; in a test build only an explicit WIN11_TEST_PULSE_SERVER, so no test ever reaches a real server.
#[cfg(not(test))]
fn server_address() -> Option<Option<String>> {
    Some(None)
}

#[cfg(test)]
fn server_address() -> Option<Option<String>> {
    std::env::var("WIN11_TEST_PULSE_SERVER")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Some)
}

/// Whether this process would talk to a PulseAudio server at all (libpulse and its GLib loop load).
pub(crate) fn possible() -> bool {
    pulse::libs().is_some() && server_address().is_some()
}

/// The process's client, started on first use; None without libpulse.
pub(crate) fn shared() -> Option<Arc<Client>> {
    CLIENT
        .get_or_init(|| {
            let server = server_address()?;
            Client::start(server)
        })
        .clone()
}

impl Client {
    fn start(server: Option<String>) -> Option<Arc<Client>> {
        let libs = pulse::libs()?;
        let server = match server {
            Some(s) => Some(CString::new(s).ok()?),
            None => None,
        };
        let context = glib::MainContext::new();
        let shared = Arc::new(Shared {
            conn: Mutex::new(Conn::Connecting),
            cond: Condvar::new(),
            snapshot: Mutex::new(None),
            listeners: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        });
        let (tc, ts) = (context.clone(), shared.clone());
        std::thread::Builder::new()
            .name("win11-sound".into())
            .spawn(move || {
                let _ = tc.with_thread_default(|| {
                    let raw: *mut glib::ffi::GMainContext = tc.to_glib_none().0;
                    let ml = unsafe { (libs.glib.pa_glib_mainloop_new)(raw.cast()) };
                    if ml.is_null() {
                        ts.set_conn(Conn::Down);
                        return;
                    }
                    let api = unsafe { (libs.glib.pa_glib_mainloop_get_api)(ml) };
                    let engine = Rc::new(Engine::new(libs, api, tc.clone(), server, ts.clone()));
                    ENGINE.with(|e| *e.borrow_mut() = Some(engine.clone()));
                    engine.connect();
                    loop {
                        tc.iteration(true);
                    }
                });
            })
            .ok()?;
        let max_volume = unsafe { (libs.pa.pa_sw_volume_from_dB)(11.0) };
        Some(Arc::new(Client {
            context,
            shared,
            max_volume,
        }))
    }

    /// GNOME's amplified maximum (PA_VOLUME_UI_MAX, +11 dB: about 153 %), in percent.
    pub(crate) fn max_volume_pct(&self) -> f64 {
        model::pct(self.max_volume)
    }

    /// Wait (at most `timeout`) for the first connection attempt to end; whether the client is connected.
    pub(crate) fn wait_ready(&self, timeout: Duration) -> bool {
        let guard = self.shared.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = self
            .shared
            .cond
            .wait_timeout_while(guard, timeout, |c| *c == Conn::Connecting)
            .unwrap_or_else(|e| e.into_inner());
        *guard == Conn::Ready
    }

    pub(crate) fn ready(&self) -> bool {
        *self.shared.conn.lock().unwrap_or_else(|e| e.into_inner()) == Conn::Ready
    }

    /// The model as last published (None while not connected).
    pub(crate) fn snapshot(&self) -> Option<Model> {
        if !self.ready() {
            return None;
        }
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn listen(&self, f: Listener) -> u64 {
        let id = self.shared.next.fetch_add(1, Ordering::Relaxed);
        self.shared
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, f));
        id
    }

    pub(crate) fn unlisten(&self, id: u64) {
        self.shared
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(i, _)| *i != id);
    }

    /// Run `job` on the sound thread and wait for its result.
    pub(crate) fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Engine) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.context.invoke(move || {
            if let Some(e) = engine() {
                let _ = tx.send(job(&e));
            }
        });
        rx.recv_timeout(WAIT).ok()
    }

    /// Run a writing job on the sound thread: it starts its libpulse operations (each [`Engine::op`]), then this
    /// waits for every one of them, in order.
    pub(crate) fn write(
        &self,
        job: impl FnOnce(&Engine) -> Result<Vec<Reply>, Fail> + Send + 'static,
    ) -> Result<(), Fail> {
        if !self.ready() {
            return Err(Fail::Unavailable);
        }
        let replies = self.run(job).ok_or(Fail::Unavailable)??;
        for r in replies {
            match r.recv_timeout(WAIT) {
                Ok(Ok(())) => {}
                Ok(Err(message)) => return Err(Fail::Failed(message)),
                Err(_) => return Err(Fail::Failed(String::new())),
            }
        }
        Ok(())
    }
}

/// The answer of one libpulse operation.
pub(crate) type Reply = mpsc::Receiver<Result<(), String>>;

struct Meter {
    stream: *mut c_void,
    kind: String,
    name: String,
    until: Instant,
    last: Instant,
}

/// The sound thread's state (only ever touched on that thread).
pub(crate) struct Engine {
    lib: &'static Libs,
    api: *mut c_void,
    context: glib::MainContext,
    server: Option<CString>,
    shared: Arc<Shared>,
    ctx: Cell<*mut c_void>,
    ready: Cell<bool>,
    loading: Cell<u32>,
    loaded: Cell<bool>,
    model: RefCell<Model>,
    flush_pending: Cell<bool>,
    retry_pending: Cell<bool>,
    retries: Cell<u32>,
    meters: RefCell<BTreeMap<u64, Meter>>,
    next_meter: Cell<u64>,
    sweeping: Cell<bool>,
}

thread_local! {
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
}

fn engine() -> Option<Rc<Engine>> {
    ENGINE.with(|e| e.borrow().clone())
}

/// Run `f` on the sound thread's context after the current dispatch (libpulse may call a state callback from inside
/// pa_context_connect or pa_stream_disconnect, where nothing may be freed).
fn defer(context: &glib::MainContext, delay: Duration, f: impl FnOnce() + Send + 'static) {
    let mut f = Some(f);
    let source =
        glib::source::timeout_source_new(delay, None, glib::Priority::DEFAULT, move || {
            if let Some(f) = f.take() {
                f();
            }
            glib::ControlFlow::Break
        });
    source.attach(Some(context));
}

/// Tag of the initial list reads (their end counts toward `loaded`), as libpulse userdata.
const INITIAL: usize = 1;

fn cstr(s: &str) -> Result<CString, Fail> {
    CString::new(s).map_err(|_| Fail::Bad)
}

fn avail(a: c_int) -> &'static str {
    match a {
        pulse::PORT_AVAILABLE_NO => "no",
        pulse::PORT_AVAILABLE_YES => "yes",
        _ => "unknown",
    }
}

unsafe fn ports(list: *mut *mut pulse::PortInfo, n: u32) -> Vec<Port> {
    let mut out = Vec::new();
    if list.is_null() {
        return out;
    }
    for i in 0..n as usize {
        let p = unsafe { *list.add(i) };
        if p.is_null() {
            continue;
        }
        let p = unsafe { &*p };
        out.push(Port {
            name: pulse::text(p.name),
            description: pulse::text(p.description),
            priority: p.priority,
            available: avail(p.available),
        });
    }
    out
}

unsafe fn profile2(p: *const pulse::CardProfileInfo2) -> Option<Profile> {
    if p.is_null() {
        return None;
    }
    let p = unsafe { &*p };
    Some(Profile {
        name: pulse::text(p.name),
        description: pulse::text(p.description),
        sinks: p.n_sinks,
        sources: p.n_sources,
        priority: p.priority,
        available: p.available != 0,
    })
}

impl Engine {
    fn new(
        lib: &'static Libs,
        api: *mut c_void,
        context: glib::MainContext,
        server: Option<CString>,
        shared: Arc<Shared>,
    ) -> Engine {
        Engine {
            lib,
            api,
            context,
            server,
            shared,
            ctx: Cell::new(ptr::null_mut()),
            ready: Cell::new(false),
            loading: Cell::new(0),
            loaded: Cell::new(false),
            model: RefCell::new(Model::default()),
            flush_pending: Cell::new(false),
            retry_pending: Cell::new(false),
            retries: Cell::new(0),
            meters: RefCell::new(BTreeMap::new()),
            next_meter: Cell::new(1),
            sweeping: Cell::new(false),
        }
    }

    fn pa(&self) -> &'static pulse::Pulse {
        &self.lib.pa
    }

    fn proplist(&self, entries: &[(&str, &str)]) -> *mut c_void {
        let pa = self.pa();
        let pl = unsafe { (pa.pa_proplist_new)() };
        for (k, v) in entries {
            if let (Ok(k), Ok(v)) = (CString::new(*k), CString::new(*v)) {
                unsafe { (pa.pa_proplist_sets)(pl, k.as_ptr(), v.as_ptr()) };
            }
        }
        pl
    }

    fn connect(&self) {
        self.retry_pending.set(false);
        let pa = self.pa();
        let pl = self.proplist(&[
            ("application.name", "WinPoint"),
            ("application.id", "win11-gnome"),
            ("application.icon_name", "multimedia-volume-control"),
        ]);
        let ctx = unsafe { (pa.pa_context_new_with_proplist)(self.api, c"WinPoint".as_ptr(), pl) };
        unsafe { (pa.pa_proplist_free)(pl) };
        if ctx.is_null() {
            self.lost();
            return;
        }
        self.ctx.set(ctx);
        unsafe { (pa.pa_context_set_state_callback)(ctx, Some(on_state), ptr::null_mut()) };
        let server = self.server.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        if unsafe { (pa.pa_context_connect)(ctx, server, pulse::CONTEXT_NOAUTOSPAWN, ptr::null()) }
            < 0
        {
            self.lost();
        }
    }

    fn state_changed(&self, c: *mut c_void) {
        if c.is_null() || c != self.ctx.get() {
            return;
        }
        match unsafe { (self.pa().pa_context_get_state)(c) } {
            pulse::CONTEXT_READY if !self.ready.get() => self.on_ready(c),
            pulse::CONTEXT_FAILED | pulse::CONTEXT_TERMINATED => self.lost(),
            _ => {}
        }
    }

    fn on_ready(&self, c: *mut c_void) {
        let pa = self.pa();
        self.ready.set(true);
        self.retries.set(0);
        *self.model.borrow_mut() = Model::default();
        let tag = INITIAL as *mut c_void;
        self.loading.set(8);
        unsafe {
            (pa.pa_context_set_subscribe_callback)(c, Some(on_event), ptr::null_mut());
            self.unref((pa.pa_context_subscribe)(
                c,
                pulse::SUBSCRIBE_MASK,
                None,
                ptr::null_mut(),
            ));
            self.initial((pa.pa_context_get_server_info)(c, Some(on_server), tag));
            self.initial((pa.pa_context_get_sink_info_list)(c, Some(on_sink), tag));
            self.initial((pa.pa_context_get_source_info_list)(
                c,
                Some(on_source),
                tag,
            ));
            self.initial((pa.pa_context_get_sink_input_info_list)(
                c,
                Some(on_sink_input),
                tag,
            ));
            self.initial((pa.pa_context_get_source_output_info_list)(
                c,
                Some(on_source_output),
                tag,
            ));
            self.initial((pa.pa_context_get_client_info_list)(
                c,
                Some(on_client),
                tag,
            ));
            self.initial((pa.pa_context_get_card_info_list)(c, Some(on_card), tag));
            (pa.pa_ext_stream_restore_set_subscribe_cb)(
                c,
                Some(on_restore_changed),
                ptr::null_mut(),
            );
            self.unref((pa.pa_ext_stream_restore_subscribe)(
                c,
                1,
                None,
                ptr::null_mut(),
            ));
            // a server without module-stream-restore answers with an error: the event role keeps its 100 %
            self.initial((pa.pa_ext_stream_restore_read)(c, Some(on_restore), tag));
        }
    }

    /// One of the initial reads started (or could not start: then it counts as ended).
    unsafe fn initial(&self, op: *mut c_void) {
        if op.is_null() {
            self.loaded_one();
        } else {
            unsafe { self.unref(op) };
        }
    }

    /// One of the initial reads ended (a list's end, or its error).
    fn loaded_one(&self) {
        let left = self.loading.get().saturating_sub(1);
        self.loading.set(left);
        if left == 0 && self.ready.get() && !self.loaded.get() {
            self.loaded.set(true);
            self.shared.set_conn(Conn::Ready);
            self.flush();
        }
    }

    /// The server is gone (or never came): drop the context and its meters, tell the listeners, try again later.
    fn lost(&self) {
        let pa = self.pa();
        let ctx = self.ctx.replace(ptr::null_mut());
        self.stop_meters();
        if !ctx.is_null() {
            unsafe {
                (pa.pa_context_set_state_callback)(ctx, None, ptr::null_mut());
                (pa.pa_context_set_subscribe_callback)(ctx, None, ptr::null_mut());
                (pa.pa_context_disconnect)(ctx);
                (pa.pa_context_unref)(ctx);
            }
        }
        let was = self.loaded.replace(false);
        self.ready.set(false);
        *self.model.borrow_mut() = Model::default();
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.shared.set_conn(Conn::Down);
        if was {
            self.shared.notify(&Event::Changed);
        }
        if !self.retry_pending.replace(true) {
            let n = self.retries.get();
            self.retries.set(n + 1);
            let seconds = [1, 2, 5, 10][(n as usize).min(3)];
            defer(&self.context, Duration::from_secs(seconds), || {
                if let Some(e) = engine() {
                    e.connect();
                }
            });
        }
    }

    unsafe fn unref(&self, op: *mut c_void) {
        if !op.is_null() {
            unsafe { (self.pa().pa_operation_unref)(op) };
        }
    }

    /// The model changed: publish it 40 ms after the last change of a burst.
    fn touch(&self) {
        if !self.loaded.get() || self.flush_pending.replace(true) {
            return;
        }
        defer(&self.context, Duration::from_millis(40), || {
            if let Some(e) = engine() {
                e.flush();
            }
        });
    }

    fn flush(&self) {
        self.flush_pending.set(false);
        if !self.loaded.get() {
            return;
        }
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(self.model.borrow().clone());
        self.shared.notify(&Event::Changed);
    }

    fn error_text(&self) -> String {
        let c = self.ctx.get();
        if c.is_null() {
            return String::new();
        }
        unsafe { pulse::text((self.pa().pa_strerror)((self.pa().pa_context_errno)(c))) }
    }

    /// Start one libpulse operation with a success callback; its answer arrives on the returned [`Reply`].
    pub(crate) fn op(
        &self,
        start: impl FnOnce(*mut c_void, pulse::SuccessCb, *mut c_void) -> *mut c_void,
    ) -> Reply {
        let (tx, rx) = mpsc::sync_channel(1);
        let c = self.ctx.get();
        if c.is_null() || !self.ready.get() {
            let _ = tx.send(Err(String::new()));
            return rx;
        }
        let ud = Box::into_raw(Box::new(tx));
        let op = start(c, Some(on_done), ud.cast());
        if op.is_null() {
            let tx = unsafe { Box::from_raw(ud) };
            let _ = tx.send(Err(self.error_text()));
        } else {
            unsafe { self.unref(op) };
        }
        rx
    }

    /// The live model (on the sound thread).
    pub(crate) fn model(&self) -> std::cell::Ref<'_, Model> {
        self.model.borrow()
    }

    fn sink(&self, name: &str) -> Result<Device, Fail> {
        self.model
            .borrow()
            .sink_named(name)
            .cloned()
            .ok_or(Fail::Gone)
    }

    fn source(&self, name: &str) -> Result<Device, Fail> {
        self.model
            .borrow()
            .source_named(name)
            .cloned()
            .ok_or(Fail::Gone)
    }

    /// The device of `kind` ("sink" / "source") named `name`, or with `index`.
    pub(crate) fn device(
        &self,
        sink: bool,
        name: Option<&str>,
        index: Option<u32>,
    ) -> Result<Device, Fail> {
        let m = self.model.borrow();
        let list = if sink { &m.sinks } else { &m.sources };
        let found = match (name, index) {
            (Some(n), _) => list.values().find(|d| d.name == n),
            (None, Some(i)) => list.get(&i),
            (None, None) => return Err(Fail::Bad),
        };
        found.cloned().ok_or(Fail::Gone)
    }

    pub(crate) fn stream(&self, output: bool, index: u32) -> Result<Stream, Fail> {
        let m = self.model.borrow();
        let list = if output {
            &m.sink_inputs
        } else {
            &m.source_outputs
        };
        list.get(&index).cloned().ok_or(Fail::Gone)
    }

    /// A cvolume scaled to `volume` (its loudest channel; pa_cvolume_scale keeps the balance, as gvc does).
    pub(crate) fn scaled(&self, values: &[u32], volume: u32) -> pulse::CVolume {
        let mut cv = pulse::CVolume::of(values);
        unsafe { (self.pa().pa_cvolume_scale)(&mut cv, volume) };
        cv
    }

    pub(crate) fn set_device_volume(&self, d: &Device, sink: bool, cv: pulse::CVolume) -> Reply {
        let pa = self.pa();
        let idx = d.index;
        self.op(|c, cb, ud| unsafe {
            if sink {
                (pa.pa_context_set_sink_volume_by_index)(c, idx, &cv, cb, ud)
            } else {
                (pa.pa_context_set_source_volume_by_index)(c, idx, &cv, cb, ud)
            }
        })
    }

    pub(crate) fn set_device_mute(&self, d: &Device, sink: bool, mute: bool) -> Reply {
        let pa = self.pa();
        let idx = d.index;
        self.op(|c, cb, ud| unsafe {
            if sink {
                (pa.pa_context_set_sink_mute_by_index)(c, idx, mute as c_int, cb, ud)
            } else {
                (pa.pa_context_set_source_mute_by_index)(c, idx, mute as c_int, cb, ud)
            }
        })
    }

    pub(crate) fn set_stream_volume(&self, s: &Stream, output: bool, cv: pulse::CVolume) -> Reply {
        let pa = self.pa();
        let idx = s.index;
        self.op(|c, cb, ud| unsafe {
            if output {
                (pa.pa_context_set_sink_input_volume)(c, idx, &cv, cb, ud)
            } else {
                (pa.pa_context_set_source_output_volume)(c, idx, &cv, cb, ud)
            }
        })
    }

    pub(crate) fn set_stream_mute(&self, s: &Stream, output: bool, mute: bool) -> Reply {
        let pa = self.pa();
        let idx = s.index;
        self.op(|c, cb, ud| unsafe {
            if output {
                (pa.pa_context_set_sink_input_mute)(c, idx, mute as c_int, cb, ud)
            } else {
                (pa.pa_context_set_source_output_mute)(c, idx, mute as c_int, cb, ud)
            }
        })
    }

    pub(crate) fn set_default(&self, sink: bool, name: &str) -> Result<Reply, Fail> {
        let pa = self.pa();
        let n = cstr(name)?;
        Ok(self.op(|c, cb, ud| unsafe {
            if sink {
                (pa.pa_context_set_default_sink)(c, n.as_ptr(), cb, ud)
            } else {
                (pa.pa_context_set_default_source)(c, n.as_ptr(), cb, ud)
            }
        }))
    }

    pub(crate) fn set_port(&self, d: &Device, sink: bool, port: &str) -> Result<Reply, Fail> {
        if !d.ports.iter().any(|p| p.name == port) {
            return Err(Fail::Bad);
        }
        let pa = self.pa();
        let (idx, p) = (d.index, cstr(port)?);
        Ok(self.op(|c, cb, ud| unsafe {
            if sink {
                (pa.pa_context_set_sink_port_by_index)(c, idx, p.as_ptr(), cb, ud)
            } else {
                (pa.pa_context_set_source_port_by_index)(c, idx, p.as_ptr(), cb, ud)
            }
        }))
    }

    /// A card's profile ("off" = Audio: Niet toestaan; the active profile before is remembered for Toestaan).
    pub(crate) fn set_profile(&self, card: &str, profile: &str) -> Result<Reply, Fail> {
        let (idx, active) = {
            let m = self.model.borrow();
            let c = m.card_named(card).ok_or(Fail::Gone)?;
            if !c.profiles.iter().any(|p| p.name == profile) {
                return Err(Fail::Bad);
            }
            (c.index, c.active_profile.clone())
        };
        if profile == "off"
            && let Some(active) = active.filter(|a| a != "off")
        {
            self.model
                .borrow_mut()
                .previous_profiles
                .insert(card.to_owned(), active);
        }
        let pa = self.pa();
        let p = cstr(profile)?;
        Ok(self.op(|c, cb, ud| unsafe {
            (pa.pa_context_set_card_profile_by_index)(c, idx, p.as_ptr(), cb, ud)
        }))
    }

    pub(crate) fn move_stream(
        &self,
        output: bool,
        index: u32,
        device: &str,
    ) -> Result<Reply, Fail> {
        self.stream(output, index)?;
        let target = if output {
            self.sink(device)?
        } else {
            self.source(device)?
        };
        if target.monitor_of.is_some() {
            return Err(Fail::Bad);
        }
        let pa = self.pa();
        Ok(self.op(|c, cb, ud| unsafe {
            if output {
                (pa.pa_context_move_sink_input_by_index)(c, index, target.index, cb, ud)
            } else {
                (pa.pa_context_move_source_output_by_index)(c, index, target.index, cb, ud)
            }
        }))
    }

    /// GNOME's "System Sounds" (gvc_mixer_event_role_push_volume): the event role's stream-restore entry with every
    /// channel at `volume`, its device and mute kept.
    pub(crate) fn set_event_volume(&self, volume: u32, mute: Option<bool>) -> Result<Reply, Fail> {
        let ev: EventRole = self.model.borrow().event.clone();
        let map = if ev.map.is_empty() {
            vec![pulse::POSITION_MONO]
        } else {
            ev.map.clone()
        };
        let name = cstr(model::EVENT_ROLE)?;
        let device = match ev.device.as_deref() {
            Some(d) => Some(cstr(d)?),
            None => None,
        };
        let info = pulse::ExtStreamRestoreInfo {
            name: name.as_ptr(),
            channel_map: pulse::ChannelMap::of(&map),
            volume: pulse::CVolume::of(&vec![volume; map.len()]),
            device: device.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            mute: mute.unwrap_or(ev.mute) as c_int,
        };
        // `const pa_ext_stream_restore_info data[]`: an array of the structs themselves, here one
        let pa = self.pa();
        Ok(self.op(|c, cb, ud| unsafe {
            (pa.pa_ext_stream_restore_write)(c, pulse::UPDATE_REPLACE, &info, 1, 1, cb, ud)
        }))
    }

    /// Balance (-1..1), fade (-1..1) and the subwoofer (LFE) of a sink or source, on `volume` when given, through
    /// libpulse's own pa_cvolume_set_balance / _set_fade / _set_position (GNOME's sliders).
    pub(crate) fn balanced(
        &self,
        d: &Device,
        volume: Option<u32>,
        balance: Option<f64>,
        fade: Option<f64>,
        lfe: Option<u32>,
    ) -> Result<pulse::CVolume, Fail> {
        let pa = self.pa();
        if d.volume.len() != d.map.len() || d.map.is_empty() {
            return Err(Fail::Bad);
        }
        let map = pulse::ChannelMap::of(&d.map);
        let mut cv = match volume {
            Some(v) => self.scaled(&d.volume, v),
            None => pulse::CVolume::of(&d.volume),
        };
        unsafe {
            if let Some(b) = balance {
                if (pa.pa_channel_map_can_balance)(&map) == 0 {
                    return Err(Fail::Bad);
                }
                (pa.pa_cvolume_set_balance)(&mut cv, &map, b.clamp(-1.0, 1.0) as f32);
            }
            if let Some(f) = fade {
                if (pa.pa_channel_map_can_fade)(&map) == 0 {
                    return Err(Fail::Bad);
                }
                (pa.pa_cvolume_set_fade)(&mut cv, &map, f.clamp(-1.0, 1.0) as f32);
            }
            if let Some(l) = lfe {
                if (pa.pa_channel_map_has_position)(&map, pulse::POSITION_LFE) == 0 {
                    return Err(Fail::Bad);
                }
                (pa.pa_cvolume_set_position)(&mut cv, &map, pulse::POSITION_LFE, l);
            }
        }
        Ok(cv)
    }

    // ------------------------------------------------------------------------------------------------ meters

    /// Start (or renew) the level meter of a sink (its monitor) or a source; `on` false stops it.
    pub(crate) fn meter(&self, sink: bool, name: &str, on: bool) -> Result<(), Fail> {
        let kind = if sink { "sink" } else { "source" };
        let existing = self
            .meters
            .borrow()
            .iter()
            .find(|(_, m)| m.kind == kind && m.name == name)
            .map(|(id, _)| *id);
        if !on {
            if let Some(id) = existing {
                self.stop_meter(id);
            }
            return Ok(());
        }
        if let Some(id) = existing {
            if let Some(m) = self.meters.borrow_mut().get_mut(&id) {
                m.until = Instant::now() + METER_LEASE;
            }
            return Ok(());
        }
        let c = self.ctx.get();
        if c.is_null() || !self.ready.get() {
            return Err(Fail::Unavailable);
        }
        let source = if sink {
            self.sink(name)?.monitor_name.ok_or(Fail::Bad)?
        } else {
            let d = self.source(name)?;
            d.name
        };
        let pa = self.pa();
        let spec = pulse::SampleSpec {
            format: pulse::SAMPLE_FLOAT32NE,
            rate: 25,
            channels: 1,
        };
        let pl = self.proplist(&[
            ("application.id", METER_APP_ID),
            ("application.name", "WinPoint"),
            ("media.name", "Peak detect"),
        ]);
        let stream = unsafe {
            (pa.pa_stream_new_with_proplist)(c, c"Peak detect".as_ptr(), &spec, ptr::null(), pl)
        };
        unsafe { (pa.pa_proplist_free)(pl) };
        if stream.is_null() {
            return Err(Fail::Failed(self.error_text()));
        }
        let id = self.next_meter.get();
        self.next_meter.set(id + 1);
        let attr = pulse::BufferAttr {
            maxlength: u32::MAX,
            tlength: 0,
            prebuf: 0,
            minreq: 0,
            fragsize: 4,
        };
        let src = cstr(&source)?;
        unsafe {
            (pa.pa_stream_set_read_callback)(
                stream,
                Some(on_meter_read),
                id as usize as *mut c_void,
            );
            (pa.pa_stream_set_state_callback)(
                stream,
                Some(on_meter_state),
                id as usize as *mut c_void,
            );
        }
        let now = Instant::now();
        self.meters.borrow_mut().insert(
            id,
            Meter {
                stream,
                kind: kind.into(),
                name: name.into(),
                until: now + METER_LEASE,
                last: now,
            },
        );
        let flags =
            pulse::STREAM_DONT_MOVE | pulse::STREAM_PEAK_DETECT | pulse::STREAM_ADJUST_LATENCY;
        if unsafe { (pa.pa_stream_connect_record)(stream, src.as_ptr(), &attr, flags) } < 0 {
            let message = self.error_text();
            self.stop_meter(id);
            return Err(Fail::Failed(message));
        }
        self.sweep();
        Ok(())
    }

    fn stop_meter(&self, id: u64) {
        let Some(m) = self.meters.borrow_mut().remove(&id) else {
            return;
        };
        let pa = self.pa();
        unsafe {
            (pa.pa_stream_set_read_callback)(m.stream, None, ptr::null_mut());
            (pa.pa_stream_set_state_callback)(m.stream, None, ptr::null_mut());
            (pa.pa_stream_disconnect)(m.stream);
            (pa.pa_stream_unref)(m.stream);
        }
        self.shared.notify(&Event::Level {
            kind: m.kind,
            name: m.name,
            peak: 0.0,
        });
    }

    pub(crate) fn stop_meters(&self) {
        let ids: Vec<u64> = self.meters.borrow().keys().copied().collect();
        for id in ids {
            self.stop_meter(id);
        }
    }

    /// Once a second while meters run: end the ones whose lease is over.
    fn sweep(&self) {
        if self.sweeping.replace(true) {
            return;
        }
        defer(&self.context, Duration::from_secs(1), || {
            let Some(e) = engine() else { return };
            e.sweeping.set(false);
            let now = Instant::now();
            let expired: Vec<u64> = e
                .meters
                .borrow()
                .iter()
                .filter(|(_, m)| m.until <= now)
                .map(|(id, _)| *id)
                .collect();
            for id in expired {
                e.stop_meter(id);
            }
            if !e.meters.borrow().is_empty() {
                e.sweep();
            }
        });
    }

    fn meter_peak(&self, id: u64, stream: *mut c_void) {
        let pa = self.pa();
        let mut data: *const c_void = ptr::null();
        let mut len: usize = 0;
        if unsafe { (pa.pa_stream_peek)(stream, &mut data, &mut len) } < 0 {
            return;
        }
        if len == 0 {
            return;
        }
        let peak = if data.is_null() || len < 4 {
            None
        } else {
            // the last float of the fragment (GNOME's level bar)
            let v = unsafe { ptr::read_unaligned(data.cast::<u8>().add(len - 4).cast::<f32>()) };
            Some((v as f64).clamp(0.0, 1.0))
        };
        unsafe { (pa.pa_stream_drop)(stream) };
        let Some(peak) = peak else { return };
        let event = {
            let mut meters = self.meters.borrow_mut();
            let Some(m) = meters.get_mut(&id) else { return };
            let now = Instant::now();
            if now.duration_since(m.last) < Duration::from_millis(35) {
                return;
            }
            m.last = now;
            Event::Level {
                kind: m.kind.clone(),
                name: m.name.clone(),
                peak: (peak * 1000.0).round() / 1000.0,
            }
        };
        self.shared.notify(&event);
    }

    // ------------------------------------------------------------------------------------------------ decode

    fn decode_device(&self, i: &pulse::DeviceInfo, sink: bool) -> Device {
        Device {
            index: i.index,
            name: pulse::text(i.name),
            description: pulse::text(i.description),
            card: (i.card != pulse::INVALID_INDEX).then_some(i.card),
            ports: unsafe { ports(i.ports, i.n_ports) },
            active_port: (!i.active_port.is_null())
                .then(|| pulse::text(unsafe { (*i.active_port).name })),
            volume: i.volume.values(),
            map: i.channel_map.positions(),
            mute: i.mute != 0,
            base_volume: i.base_volume,
            decibel: i.flags & pulse::FLAG_DECIBEL_VOLUME != 0,
            hw_volume: i.flags & pulse::FLAG_HW_VOLUME_CTRL != 0,
            monitor_name: if sink {
                pulse::opt_text(i.monitor_name)
            } else {
                None
            },
            monitor_of: (!sink && i.monitor != pulse::INVALID_INDEX).then_some(i.monitor),
            state: i.state,
            props: pulse::props(self.pa(), i.proplist),
        }
    }

    fn decode_stream(
        &self,
        index: u32,
        name: *const libc::c_char,
        client: u32,
        device: u32,
        volume: &pulse::CVolume,
        map: &pulse::ChannelMap,
        flags: (c_int, c_int, c_int, c_int),
        proplist: *mut c_void,
    ) -> Stream {
        let (mute, corked, has_volume, writable) = flags;
        Stream {
            index,
            name: pulse::text(name),
            client: (client != pulse::INVALID_INDEX).then_some(client),
            device,
            volume: volume.values(),
            map: map.positions(),
            mute: mute != 0,
            corked: corked != 0,
            has_volume: has_volume != 0,
            volume_writable: writable != 0,
            props: pulse::props(self.pa(), proplist),
        }
    }

    fn decode_card(&self, i: &pulse::CardInfo) -> model::Card {
        let mut profiles = Vec::new();
        if !i.profiles2.is_null() {
            for k in 0..i.n_profiles as usize {
                if let Some(p) = unsafe { profile2(*i.profiles2.add(k)) } {
                    profiles.push(p);
                }
            }
        }
        let mut card_ports = Vec::new();
        if !i.ports.is_null() {
            for k in 0..i.n_ports as usize {
                let p = unsafe { *i.ports.add(k) };
                if p.is_null() {
                    continue;
                }
                let p = unsafe { &*p };
                let mut names = Vec::new();
                if !p.profiles2.is_null() {
                    for j in 0..p.n_profiles as usize {
                        if let Some(pr) = unsafe { profile2(*p.profiles2.add(j)) } {
                            names.push(pr.name);
                        }
                    }
                }
                card_ports.push(CardPort {
                    name: pulse::text(p.name),
                    description: pulse::text(p.description),
                    priority: p.priority,
                    available: avail(p.available),
                    output: p.direction == pulse::DIRECTION_OUTPUT,
                    profiles: names,
                });
            }
        }
        model::Card {
            index: i.index,
            name: pulse::text(i.name),
            driver: pulse::text(i.driver),
            profiles,
            active_profile: unsafe { profile2(i.active_profile2) }.map(|p| p.name),
            ports: card_ports,
            props: pulse::props(self.pa(), i.proplist),
        }
    }
}

// ------------------------------------------------------------------------------------------------ callbacks

/// The engine, when `c` is its current context.
fn current(c: *mut c_void) -> Option<Rc<Engine>> {
    engine().filter(|e| !c.is_null() && e.ctx.get() == c)
}

unsafe extern "C" fn on_state(c: *mut c_void, _ud: *mut c_void) {
    let addr = c as usize;
    if let Some(e) = engine() {
        defer(&e.context, Duration::ZERO, move || {
            if let Some(e) = engine() {
                e.state_changed(addr as *mut c_void);
            }
        });
    }
}

unsafe extern "C" fn on_done(_c: *mut c_void, ok: c_int, ud: *mut c_void) {
    if ud.is_null() {
        return;
    }
    let tx = unsafe { Box::from_raw(ud.cast::<mpsc::SyncSender<Result<(), String>>>()) };
    let result = if ok != 0 {
        Ok(())
    } else {
        Err(engine().map(|e| e.error_text()).unwrap_or_default())
    };
    let _ = tx.send(result);
}

unsafe extern "C" fn on_event(c: *mut c_void, t: c_int, idx: u32, _ud: *mut c_void) {
    let Some(e) = current(c) else { return };
    let pa = e.pa();
    let remove = t & pulse::TYPE_MASK == pulse::EVENT_REMOVE;
    let zero = ptr::null_mut();
    macro_rules! removed {
        ($field:ident) => {{
            e.model.borrow_mut().$field.remove(&idx);
            e.touch();
        }};
    }
    unsafe {
        match t & pulse::FACILITY_MASK {
            pulse::FACILITY_SINK if remove => removed!(sinks),
            pulse::FACILITY_SINK => e.unref((pa.pa_context_get_sink_info_by_index)(
                c,
                idx,
                Some(on_sink),
                zero,
            )),
            pulse::FACILITY_SOURCE if remove => removed!(sources),
            pulse::FACILITY_SOURCE => e.unref((pa.pa_context_get_source_info_by_index)(
                c,
                idx,
                Some(on_source),
                zero,
            )),
            pulse::FACILITY_SINK_INPUT if remove => removed!(sink_inputs),
            pulse::FACILITY_SINK_INPUT => e.unref((pa.pa_context_get_sink_input_info)(
                c,
                idx,
                Some(on_sink_input),
                zero,
            )),
            pulse::FACILITY_SOURCE_OUTPUT if remove => removed!(source_outputs),
            pulse::FACILITY_SOURCE_OUTPUT => e.unref((pa.pa_context_get_source_output_info)(
                c,
                idx,
                Some(on_source_output),
                zero,
            )),
            pulse::FACILITY_CLIENT if remove => removed!(clients),
            pulse::FACILITY_CLIENT => e.unref((pa.pa_context_get_client_info)(
                c,
                idx,
                Some(on_client),
                zero,
            )),
            pulse::FACILITY_CARD if remove => removed!(cards),
            pulse::FACILITY_CARD => e.unref((pa.pa_context_get_card_info_by_index)(
                c,
                idx,
                Some(on_card),
                zero,
            )),
            pulse::FACILITY_SERVER => {
                e.unref((pa.pa_context_get_server_info)(c, Some(on_server), zero))
            }
            _ => {}
        }
    }
}

/// A list callback's end (eol > 0) or error (eol < 0, also "no such entity" for an object that went away).
fn list_end(e: &Engine, eol: c_int, ud: *mut c_void) -> bool {
    if eol != 0 {
        if ud as usize == INITIAL {
            e.loaded_one();
        }
        return true;
    }
    false
}

unsafe extern "C" fn on_server(c: *mut c_void, i: *const pulse::ServerInfo, ud: *mut c_void) {
    let Some(e) = current(c) else { return };
    if !i.is_null() {
        let i = unsafe { &*i };
        e.model.borrow_mut().server = model::Server {
            name: pulse::text(i.server_name),
            version: pulse::text(i.server_version),
            default_sink: pulse::text(i.default_sink_name),
            default_source: pulse::text(i.default_source_name),
        };
        e.touch();
    }
    if ud as usize == INITIAL {
        e.loaded_one();
    }
}

unsafe extern "C" fn on_sink(
    c: *mut c_void,
    i: *const pulse::DeviceInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let d = e.decode_device(unsafe { &*i }, true);
    e.model.borrow_mut().sinks.insert(d.index, d);
    e.touch();
}

unsafe extern "C" fn on_source(
    c: *mut c_void,
    i: *const pulse::DeviceInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let d = e.decode_device(unsafe { &*i }, false);
    e.model.borrow_mut().sources.insert(d.index, d);
    e.touch();
}

unsafe extern "C" fn on_sink_input(
    c: *mut c_void,
    i: *const pulse::SinkInputInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let i = unsafe { &*i };
    let s = e.decode_stream(
        i.index,
        i.name,
        i.client,
        i.sink,
        &i.volume,
        &i.channel_map,
        (i.mute, i.corked, i.has_volume, i.volume_writable),
        i.proplist,
    );
    e.model.borrow_mut().sink_inputs.insert(s.index, s);
    e.touch();
}

unsafe extern "C" fn on_source_output(
    c: *mut c_void,
    i: *const pulse::SourceOutputInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let i = unsafe { &*i };
    let s = e.decode_stream(
        i.index,
        i.name,
        i.client,
        i.source,
        &i.volume,
        &i.channel_map,
        (i.mute, i.corked, i.has_volume, i.volume_writable),
        i.proplist,
    );
    e.model.borrow_mut().source_outputs.insert(s.index, s);
    e.touch();
}

unsafe extern "C" fn on_client(
    c: *mut c_void,
    i: *const pulse::ClientInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let i = unsafe { &*i };
    let client = model::Client {
        index: i.index,
        name: pulse::text(i.name),
        props: pulse::props(e.pa(), i.proplist),
    };
    e.model.borrow_mut().clients.insert(client.index, client);
    e.touch();
}

unsafe extern "C" fn on_card(
    c: *mut c_void,
    i: *const pulse::CardInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let card = e.decode_card(unsafe { &*i });
    e.model.borrow_mut().cards.insert(card.index, card);
    e.touch();
}

unsafe extern "C" fn on_restore_changed(c: *mut c_void, _ud: *mut c_void) {
    let Some(e) = current(c) else { return };
    unsafe {
        e.unref((e.pa().pa_ext_stream_restore_read)(
            c,
            Some(on_restore),
            ptr::null_mut(),
        ))
    };
}

unsafe extern "C" fn on_restore(
    c: *mut c_void,
    i: *const pulse::ExtStreamRestoreInfo,
    eol: c_int,
    ud: *mut c_void,
) {
    let Some(e) = current(c) else { return };
    if list_end(&e, eol, ud) || i.is_null() {
        return;
    }
    let i = unsafe { &*i };
    if pulse::text(i.name) != model::EVENT_ROLE {
        return;
    }
    e.model.borrow_mut().event = EventRole {
        volume: i.volume.values(),
        map: i.channel_map.positions(),
        mute: i.mute != 0,
        device: pulse::opt_text(i.device).filter(|d| !d.is_empty()),
        stored: true,
    };
    e.touch();
}

unsafe extern "C" fn on_meter_read(s: *mut c_void, _nbytes: usize, ud: *mut c_void) {
    if let Some(e) = engine() {
        e.meter_peak(ud as usize as u64, s);
    }
}

unsafe extern "C" fn on_meter_state(s: *mut c_void, ud: *mut c_void) {
    let Some(e) = engine() else { return };
    let state = unsafe { (e.pa().pa_stream_get_state)(s) };
    if state == pulse::STREAM_FAILED || state == pulse::STREAM_TERMINATED {
        let id = ud as usize as u64;
        defer(&e.context, Duration::ZERO, move || {
            if let Some(e) = engine() {
                e.stop_meter(id);
            }
        });
    }
}
