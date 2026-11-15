//! WinPoint's notifications (D-G74a, D-G74): "new version" (owner only, after an automatic check) and "installed"
//! (after a job of the owner installed it). [`Notifier::show`] says whether GNOME took the notification, so a
//! version counts as announced only once it was really shown (check.rs).
//!
//! * One `winpoint-notify` thread at a time, with its own `MainContext`. It opens a **private** session-bus
//!   connection inside that context (the shared singleton is weak and would deliver signals elsewhere), so
//!   `ActivationToken`, `ActionInvoked` and `NotificationClosed` arrive on this thread.
//! * `Notify`: app name "WinPoint", icon `system-software-update`, `replaces_id` the previous notification's id,
//!   actions `default` and `view`, hint `desktop-entry` Settings. A newer version while the thread lives replaces
//!   the shown notification.
//! * `default` or `view` on our id runs the action here, on this worker thread: Settings › Update & recovery
//!   (`shell.openApp {app:'settings', params:{page:'update'}, activationToken}`), so the launcher's wait never
//!   stalls a main loop. The thread ends then, on `NotificationClosed` of our id, or when the host shuts down.
use gio::prelude::*;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    time::Duration,
};

const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
const DESKTOP_ENTRY: &str = "io.github.win11shell.Settings";

/// One notification's texts (already in the user's language).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    pub title: String,
    pub body: String,
    pub action: String,
}

/// The notification's action: open Settings › Update & recovery with the activation token, if one came.
pub(crate) type Open = Arc<dyn Fn(Option<String>) + Send + Sync>;

/// Whether a notification was shown: the server's answer to `Notify` (a new id), at most [`SENT_WAIT`] away.
pub(crate) struct Sent(mpsc::Receiver<bool>);
impl Sent {
    /// Wait for the answer: true only when the server gave the notification an id.
    pub(crate) fn wait(self) -> bool {
        self.0.recv_timeout(SENT_WAIT).unwrap_or(false)
    }
    fn now(sent: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(sent);
        Self(rx)
    }
}
/// The `Notify` call's own timeout (5 s) plus opening the private connection.
const SENT_WAIT: Duration = Duration::from_secs(10);

/// A note on its way to the thread, with the channel its answer goes to.
type Queued = (Note, mpsc::Sender<bool>);

/// Where notifications go.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// the session bus's `org.freedesktop.Notifications` (GNOME Shell)
    Session,
    /// a test: this bus address and this well-known name
    #[cfg_attr(not(test), allow(dead_code))]
    Bus { address: String, name: String },
    /// none (dry runs and the unit tests of the service)
    Off,
}

pub(crate) struct Notifier {
    target: Target,
    sender: Arc<Mutex<Option<mpsc::Sender<Queued>>>>,
    last_id: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    /// what a test notifier was asked to show
    #[cfg(test)]
    pub shown: Mutex<Vec<Note>>,
    /// a test: [`Target::Off`] answers that the notification could not be shown
    #[cfg(test)]
    pub fail: AtomicBool,
}

impl Notifier {
    pub(crate) fn new(target: Target) -> Self {
        Self {
            target,
            sender: Arc::default(),
            last_id: Arc::default(),
            stop: Arc::default(),
            #[cfg(test)]
            shown: Mutex::default(),
            #[cfg(test)]
            fail: AtomicBool::new(false),
        }
    }
    /// Show `note`: on the running thread when there is one (it replaces the notification), else on a new one.
    /// The answer says whether it was shown (no session bus, no notification server, a refused call: not).
    pub(crate) fn show(&self, note: Note, open: Open) -> Sent {
        #[cfg(test)]
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(note.clone());
        let (address, name) = match &self.target {
            #[cfg(test)]
            Target::Off => return Sent::now(!self.fail.load(Ordering::SeqCst)),
            #[cfg(not(test))]
            Target::Off => return Sent::now(false),
            Target::Session => (None, IFACE.to_owned()),
            Target::Bus { address, name } => (Some(address.clone()), name.clone()),
        };
        let (ack, sent) = mpsc::channel();
        let mut sender = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        let note = match sender.as_ref() {
            Some(tx) => match tx.send((note, ack)) {
                Ok(()) => return Sent(sent),
                Err(mpsc::SendError(queued)) => queued,
            },
            None => (note, ack),
        };
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(note);
        *sender = Some(tx);
        drop(sender);
        let (slot, last_id, stop) = (self.sender.clone(), self.last_id.clone(), self.stop.clone());
        let spawned = std::thread::Builder::new()
            .name("winpoint-notify".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let served = context.with_thread_default(|| {
                    serve(&context, address, &name, &rx, &slot, &last_id, &stop, &open);
                });
                if served.is_err() {
                    slot.lock().unwrap_or_else(|e| e.into_inner()).take();
                }
            });
        if spawned.is_err() {
            // the note (and its answer channel) went with the closure: not shown
            self.sender.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
        Sent(sent)
    }
    pub(crate) fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.sender.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

#[derive(Default)]
struct Seen {
    tokens: HashMap<u32, String>,
    actions: Vec<(u32, String)>,
    closed: Vec<u32>,
}

/// The `Notify` call; the new id (0 when it failed).
fn send(conn: &gio::DBusConnection, name: &str, note: &Note, replaces: u32) -> u32 {
    let hints = glib::VariantDict::new(None);
    hints.insert("desktop-entry", DESKTOP_ENTRY);
    let actions = vec![
        "default".to_owned(),
        String::new(),
        "view".to_owned(),
        note.action.clone(),
    ];
    let args = glib::Variant::tuple_from_iter([
        "WinPoint".to_variant(),
        replaces.to_variant(),
        "system-software-update".to_variant(),
        note.title.to_variant(),
        note.body.to_variant(),
        actions.to_variant(),
        hints.end(),
        (-1i32).to_variant(),
    ]);
    conn.call_sync(
        Some(name),
        PATH,
        IFACE,
        "Notify",
        Some(&args),
        Some(glib::VariantTy::new("(u)").expect("constant type")),
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )
    .ok()
    .and_then(|reply| reply.child_value(0).get::<u32>())
    .unwrap_or(0)
}

type Slot = Mutex<Option<mpsc::Sender<Queued>>>;

/// The thread is done: unless a note came in meanwhile (then it is returned and served), the slot is cleared
/// under its lock, so the next `show()` starts a new thread and no note is lost between the two.
fn finish(slot: &Slot, rx: &mpsc::Receiver<Queued>) -> Option<Queued> {
    let mut sender = slot.lock().unwrap_or_else(|e| e.into_inner());
    match rx.try_recv() {
        Ok(note) => Some(note),
        Err(_) => {
            sender.take();
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn serve(
    context: &glib::MainContext,
    address: Option<String>,
    name: &str,
    rx: &mpsc::Receiver<Queued>,
    slot: &Slot,
    last_id: &AtomicU32,
    stop: &AtomicBool,
    open: &Open,
) {
    let address = match address {
        Some(address) => Some(address),
        None => gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .ok()
            .map(|a| a.to_string()),
    };
    let conn = address.and_then(|address| {
        gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .ok()
    });
    let Some(conn) = conn else {
        // no session bus: nothing can be shown (the notes waiting now go with it)
        slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        return;
    };
    let seen = Rc::new(RefCell::new(Seen::default()));
    let subscription = {
        let seen = seen.clone();
        conn.signal_subscribe(
            Some(name),
            Some(IFACE),
            None,
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, member, params| {
                let id = params
                    .try_child_value(0)
                    .and_then(|v| v.get::<u32>())
                    .unwrap_or(0);
                let text = || {
                    params
                        .try_child_value(1)
                        .and_then(|v| v.str().map(str::to_owned))
                };
                let mut seen = seen.borrow_mut();
                match member {
                    "ActivationToken" => {
                        if let Some(token) = text().filter(|t| t.len() < 512) {
                            seen.tokens.insert(id, token);
                        }
                    }
                    "ActionInvoked" => {
                        if let Some(key) = text() {
                            seen.actions.push((id, key));
                        }
                    }
                    "NotificationClosed" => seen.closed.push(id),
                    _ => {}
                }
            },
        )
    };
    // wakes the loop once a second for a new note and the shutdown flag
    let tick = glib::timeout_source_new(
        Duration::from_secs(1),
        None,
        glib::Priority::DEFAULT,
        || glib::ControlFlow::Continue,
    );
    tick.attach(Some(context));
    let mut id = 0u32;
    let mut next = None;
    loop {
        while let Some((note, ack)) = next.take().or_else(|| rx.try_recv().ok()) {
            id = send(&conn, name, &note, last_id.load(Ordering::SeqCst));
            if id != 0 {
                last_id.store(id, Ordering::SeqCst);
            }
            let _ = ack.send(id != 0);
        }
        let (actions, closed, tokens) = {
            let mut seen = seen.borrow_mut();
            (
                std::mem::take(&mut seen.actions),
                std::mem::take(&mut seen.closed),
                seen.tokens.clone(),
            )
        };
        let ours = actions
            .iter()
            .any(|(o, key)| *o == id && (key == "default" || key == "view"));
        if ours {
            open(tokens.get(&id).cloned());
        }
        // done: answered, closed, not shown at all (no server), or the host shuts down
        if ours || id == 0 || closed.contains(&id) || stop.load(Ordering::SeqCst) {
            next = finish(slot, rx);
            if next.is_none() || stop.load(Ordering::SeqCst) {
                break;
            }
            continue;
        }
        context.iteration(true);
    }
    tick.destroy();
    conn.signal_unsubscribe(subscription);
    let _ = conn.close_sync(gio::Cancellable::NONE);
}
