//! On-demand xdg-desktop-portal FileChooser backend using the existing native Explorer picker.
mod external;
mod model;
#[cfg(test)]
mod tests;
use glib::variant::ToVariant;
use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
const NAME: &str = "org.freedesktop.impl.portal.desktop.win11shell";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.impl.portal.Request";
const MAX_PENDING: usize = 4;
enum Phase {
    Preparing(mpsc::Receiver<Value>, Instant),
    Choosing(mpsc::Receiver<Result<Value, String>>),
    Resolving(
        mpsc::Receiver<Result<Option<BTreeMap<String, glib::Variant>>, String>>,
        Instant,
    ),
}
struct Pending {
    owner: String,
    request: model::Request,
    invocation: gio::DBusMethodInvocation,
    registration: gio::RegistrationId,
    phase: Phase,
    key: Option<String>,
    parent: Option<gtk::ApplicationWindow>,
    external: Option<Rc<external::Parent>>,
}
struct Portal {
    app: glib::WeakRef<gtk::Application>,
    bus: gio::DBusConnection,
    runtime: Arc<crate::runtime::Runtime>,
    address: String,
    pending: RefCell<BTreeMap<String, Pending>>,
    activity: Cell<Instant>,
    workers: Arc<AtomicUsize>,
    registration: RefCell<Option<gio::RegistrationId>>,
}
// gio 0.18 requires Send+Sync handlers, but dispatches them on the registration context.
// GTK objects and method invocations stay on that thread; callbacks carry no GTK/Rc value.
thread_local! {static CURRENT:RefCell<std::rc::Weak<Portal>>=RefCell::new(std::rc::Weak::new());}
fn current() -> Option<Rc<Portal>> {
    CURRENT.with(|value| value.borrow().upgrade())
}
fn trace(phase: &str) {
    if std::env::var_os("WIN11_FILE_PORTAL_TRACE").is_some() {
        eprintln!("Win11 file portal: {phase}");
    }
}
impl Portal {
    fn register(self: &Rc<Self>) -> Result<(), String> {
        let info = gio::DBusNodeInfo::for_xml(include_str!("portal/filechooser.xml"))
            .map_err(|e| e.to_string())?
            .lookup_interface(IFACE)
            .ok_or("Portal XML interface missing")?;
        CURRENT.with(|value| *value.borrow_mut() = Rc::downgrade(self));
        let registration = self
            .bus
            .register_object(
                PATH,
                &info,
                move |_, sender, _, _, method, params, inv| {
                    let Some(portal) = current() else {
                        inv.return_dbus_error(
                            "org.freedesktop.DBus.Error.Failed",
                            "Portal stopped",
                        );
                        return;
                    };
                    portal.open(sender, method, &params, inv);
                },
                |_, _, _, _, _| ().to_variant(),
                |_, _, _, _, _, _| false,
            )
            .map_err(|e| e.to_string())?;
        *self.registration.borrow_mut() = Some(registration);
        Ok(())
    }
    fn open(
        self: &Rc<Self>,
        sender: &str,
        method: &str,
        params: &glib::Variant,
        inv: gio::DBusMethodInvocation,
    ) {
        if !["OpenFile", "SaveFile", "SaveFiles"].contains(&method) {
            inv.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "Unknown FileChooser method",
            );
            return;
        }
        let request = match model::Request::parse(method, params) {
            Ok(r) => r,
            Err(e) => {
                inv.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", &e);
                return;
            }
        };
        if self.pending.borrow().contains_key(&request.handle) {
            inv.return_dbus_error(
                "org.freedesktop.DBus.Error.InvalidArgs",
                "Request already exists",
            );
            return;
        }
        if self.pending.borrow().len() >= MAX_PENDING
            || self.workers.load(Ordering::Relaxed) >= MAX_PENDING
        {
            inv.return_dbus_error(
                "org.freedesktop.DBus.Error.LimitsExceeded",
                "Too many file chooser requests",
            );
            return;
        }
        let info = gio::DBusNodeInfo::for_xml(include_str!("portal/request.xml"))
            .unwrap()
            .lookup_interface(REQUEST)
            .unwrap();
        let handle = request.handle.clone();
        let reg = self.bus.register_object(
            &request.handle,
            &info,
            move |_, sender, _, _, method, _, inv| {
                let Some(portal) = current() else {
                    inv.return_dbus_error("org.freedesktop.DBus.Error.Failed", "Portal stopped");
                    return;
                };
                if method != "Close" {
                    inv.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Unknown request method",
                    );
                    return;
                }
                let allowed = portal
                    .pending
                    .borrow()
                    .get(&handle)
                    .is_some_and(|p| p.owner == sender);
                if !allowed {
                    inv.return_dbus_error(
                        "org.freedesktop.DBus.Error.AccessDenied",
                        "Request belongs to another caller",
                    );
                    return;
                }
                portal.finish(&handle, 2, None);
                inv.return_value(None);
            },
            |_, _, _, _, _| ().to_variant(),
            |_, _, _, _, _, _| false,
        );
        let reg = match reg {
            Ok(r) => r,
            Err(e) => {
                inv.return_dbus_error("org.freedesktop.DBus.Error.Failed", &e.to_string());
                return;
            }
        };
        let (tx, rx) = mpsc::sync_channel(1);
        let prepare = request.clone();
        let workers = self.workers.clone();
        workers.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            let value = prepare.picker();
            workers.fetch_sub(1, Ordering::Relaxed);
            let _ = tx.send(value);
        });
        self.activity.set(Instant::now());
        self.pending.borrow_mut().insert(
            request.handle.clone(),
            Pending {
                owner: sender.into(),
                request,
                invocation: inv,
                registration: reg,
                phase: Phase::Preparing(rx, Instant::now()),
                key: None,
                parent: None,
                external: None,
            },
        );
    }
    fn choose(self: &Rc<Self>, handle: &str, options: Value) {
        let Some(app) = self.app.upgrade() else {
            self.finish(handle, 2, None);
            return;
        };
        let Some(request) = self.pending.borrow().get(handle).map(|p| p.request.clone()) else {
            return;
        };
        let external = match external::Parent::new(&request.parent) {
            Ok(p) => Rc::new(p),
            Err(e) => {
                eprintln!("Win11 file portal: {e}; opening without external parent");
                match external::Parent::new("") {
                    Ok(p) => Rc::new(p),
                    Err(_) => {
                        self.finish(handle, 2, None);
                        return;
                    }
                }
            }
        };
        let parent = gtk::ApplicationWindow::builder()
            .application(&app)
            .screen(&external.display.default_screen())
            .title("Win11 FileChooser parent")
            .build();
        let (tx, rx) = mpsc::channel();
        let transient = external.clone();
        let configure = Rc::new(move |window: &gtk::ApplicationWindow| {
            if window.window().is_some() {
                if let Err(e) = transient.apply(window) {
                    eprintln!("Win11 file portal: {e}");
                }
            } else {
                let transient = transient.clone();
                window.connect_realize(move |window| {
                    if let Err(e) = transient.apply(window) {
                        eprintln!("Win11 file portal: {e}");
                    }
                });
            }
        });
        let key = match crate::picker::show_portal_with_parent(
            &parent,
            options,
            &self.address,
            tx,
            configure,
        ) {
            Ok(key) => key,
            Err(e) => {
                eprintln!("Win11 file portal: {e}");
                unsafe {
                    parent.destroy();
                }
                self.finish(handle, 2, None);
                return;
            }
        };
        if let Some(window) = crate::picker::window(&key) {
            // Like the GTK reference backend, present the completed chooser after
            // showing it. show_all alone does not activate an imported parent's modal.
            window.present();
        }
        if let Some(pending) = self.pending.borrow_mut().get_mut(handle) {
            pending.key = Some(key);
            pending.parent = Some(parent);
            pending.external = Some(external);
            pending.phase = Phase::Choosing(rx);
        }
    }
    fn resolve(self: &Rc<Self>, handle: &str, value: Value) {
        if self.workers.load(Ordering::Relaxed) >= MAX_PENDING {
            self.finish(handle, 2, None);
            return;
        }
        let Some(request) = self.pending.borrow().get(handle).map(|p| p.request.clone()) else {
            return;
        };
        let (tx, rx) = mpsc::sync_channel(1);
        let workers = self.workers.clone();
        workers.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            let result = request.result(&value);
            workers.fetch_sub(1, Ordering::Relaxed);
            let _ = tx.send(result);
        });
        if let Some(pending) = self.pending.borrow_mut().get_mut(handle) {
            pending.phase = Phase::Resolving(rx, Instant::now());
        }
    }
    fn poll(self: &Rc<Self>) {
        enum Update {
            Choose(String, Value),
            Resolve(String, Value),
            Finish(String, u32, Option<BTreeMap<String, glib::Variant>>),
        }
        let mut updates = vec![];
        {
            let pending = self.pending.borrow();
            for (handle, pending) in pending.iter() {
                match &pending.phase {
                    Phase::Preparing(rx, at) => match rx.try_recv() {
                        Ok(v) => updates.push(Update::Choose(handle.clone(), v)),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            updates.push(Update::Finish(handle.clone(), 2, None))
                        }
                        _ if at.elapsed() > Duration::from_secs(5) => {
                            updates.push(Update::Finish(handle.clone(), 2, None))
                        }
                        _ => {}
                    },
                    Phase::Choosing(rx) => match rx.try_recv() {
                        Ok(Ok(v)) => updates.push(Update::Resolve(handle.clone(), v)),
                        Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                            updates.push(Update::Finish(handle.clone(), 1, None))
                        }
                        _ => {}
                    },
                    Phase::Resolving(rx, at) => match rx.try_recv() {
                        Ok(Ok(Some(v))) => updates.push(Update::Finish(handle.clone(), 0, Some(v))),
                        Ok(Ok(None)) => updates.push(Update::Finish(handle.clone(), 1, None)),
                        Ok(Err(e)) => {
                            eprintln!("Win11 file portal: {e}");
                            updates.push(Update::Finish(handle.clone(), 2, None));
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            updates.push(Update::Finish(handle.clone(), 2, None))
                        }
                        _ if at.elapsed() > Duration::from_secs(5) => {
                            updates.push(Update::Finish(handle.clone(), 2, None))
                        }
                        _ => {}
                    },
                }
            }
        }
        for update in updates {
            match update {
                Update::Choose(h, v) => self.choose(&h, v),
                Update::Resolve(h, v) => self.resolve(&h, v),
                Update::Finish(h, c, v) => self.finish(&h, c, v),
            }
        }
    }
    fn finish(
        &self,
        handle: &str,
        response: u32,
        results: Option<BTreeMap<String, glib::Variant>>,
    ) {
        let pending = self.pending.borrow_mut().remove(handle);
        let Some(pending) = pending else {
            return;
        };
        let _ = self.bus.unregister_object(pending.registration);
        pending
            .invocation
            .return_value(Some(&(response, results.unwrap_or_default()).to_variant()));
        if let Some(key) = pending.key {
            crate::picker::cancel(&key);
        }
        if let Some(parent) = pending.parent {
            unsafe {
                parent.destroy();
            }
        }
        self.activity.set(Instant::now());
    }
    fn close_owner(&self, owner: &str) {
        let keys = self
            .pending
            .borrow()
            .iter()
            .filter(|(_, p)| p.owner == owner)
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.finish(&key, 2, None);
        }
    }
    fn shutdown(&self) {
        if let Some(reg) = self.registration.borrow_mut().take() {
            let _ = self.bus.unregister_object(reg);
        }
        let keys = self.pending.borrow().keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.finish(&key, 2, None);
        }
        self.runtime.shutdown();
    }
}
pub(crate) fn run() -> Result<(), String> {
    trace("entry");
    // This process implements a portal. GTK must read its native desktop settings
    // directly, otherwise a frontend activating us can wait on itself in gtk_init.
    // run() is the executable entry point; no worker thread exists at this point.
    unsafe {
        if std::env::var_os("WIN11_PORTAL_ENV").is_none() {
            if let Some(value) = std::env::var_os("GTK_USE_PORTAL") {
                std::env::set_var("WIN11_PORTAL_PARENT_GTK_USE_PORTAL", value);
            } else {
                std::env::remove_var("WIN11_PORTAL_PARENT_GTK_USE_PORTAL");
            }
            std::env::set_var("WIN11_PORTAL_ENV", "1");
        }
        std::env::set_var("GTK_USE_PORTAL", "0");
    }
    if crate::process_scope::ensure_native("file-portal")? {
        return Ok(());
    }
    trace("native scope ready");
    glib::set_prgname(Some(NAME));
    gtk::init().map_err(|e| e.to_string())?;
    trace("GTK ready");
    // GApplication acquires its ID before startup. Keep that ID private to this
    // host and publish the portal implementation only after its object is ready.
    let app = gtk::Application::new(
        Some("io.github.win11shell.FilePortal"),
        gio::ApplicationFlags::IS_SERVICE,
    );
    let _hold = app.hold();
    let slot = Rc::new(RefCell::new(None::<Rc<Portal>>));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let instance = std::env::var_os("WIN11_NATIVE_INSTANCE").map(PathBuf::from);
    let discover = instance.clone();
    // Publish the ready implementation before GtkApplication's default startup,
    // which may synchronously inspect the frontend. The frontend itself can be
    // waiting for this implementation's activation. Callbacks run once app.run
    // starts the registration context; no picker is created before GTK startup.
    let setup = (|| -> Result<Rc<Portal>, String> {
        let (ui_tx, ui_rx) = mpsc::channel();
        let lifecycle = ui_tx.clone();
        let runtime = crate::runtime::Runtime::new(
            crate::ui_root()?,
            "picker",
            ui_tx,
            Arc::new(move |restart| {
                let (ack, _) = mpsc::channel();
                let _ = lifecycle.send(crate::UiRequest {
                    kind: crate::UiRequestKind::Lifecycle,
                    payload: json!({"restart":restart}),
                    ack,
                });
            }),
            true,
        )?;
        trace("runtime ready");
        let (port, server_stop) = runtime.server()?;
        let address = format!(
            "http://127.0.0.1:{port}/ui/index.html?token={}",
            runtime.backend.token
        );
        if let Some(path) = &discover {
            crate::backend::write_instance(path, port, &runtime.backend.token)?;
        }
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .map_err(|e| e.to_string())?;
        let portal = Rc::new(Portal {
            app: app.downgrade(),
            bus: bus.clone(),
            runtime,
            address,
            pending: RefCell::new(BTreeMap::new()),
            activity: Cell::new(Instant::now()),
            workers: Arc::new(AtomicUsize::new(0)),
            registration: RefCell::new(None),
        });
        portal.register()?;
        trace("FileChooser registered");
        let acquired = bus
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&(NAME, 4u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
            )
            .map_err(|e| e.to_string())?
            .get::<(u32,)>()
            .is_some_and(|(code,)| code == 1 || code == 4);
        if !acquired {
            portal.shutdown();
            return Err("Another process already owns the Win11 FileChooser backend".into());
        }
        trace("backend name acquired");
        let weak = Rc::downgrade(&portal);
        bus.signal_subscribe(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, params| {
                if let Some((name, _, new)) = params.get::<(String, String, String)>() {
                    if name.starts_with(':') && new.is_empty() {
                        if let Some(portal) = weak.upgrade() {
                            portal.close_owner(&name);
                        }
                    }
                }
            },
        );
        let weak = Rc::downgrade(&portal);
        let quitting = stopping.clone();
        let idle = std::env::var("WIN11_FILE_PORTAL_IDLE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n > 0.)
            .unwrap_or(15.);
        let idle = Duration::from_secs_f64(idle.clamp(0.2, 300.));
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(portal) = weak.upgrade() else {
                server_stop.store(true, Ordering::Relaxed);
                return glib::ControlFlow::Break;
            };
            if !portal.app.upgrade().is_some_and(|app| app.is_registered()) {
                return glib::ControlFlow::Continue;
            }
            portal.runtime.beat();
            portal.poll();
            for request in ui_rx.try_iter().take(8) {
                match request.kind {
                    crate::UiRequestKind::Lifecycle => {
                        let _ = request.ack.send(Ok(json!({})));
                        if let Some(app) = portal.app.upgrade() {
                            app.quit();
                        }
                    }
                    _ => {
                        let _ = request
                            .ack
                            .send(Err("Nested file chooser unavailable in portal".into()));
                    }
                }
            }
            if quitting.load(Ordering::Relaxed)
                || (portal.pending.borrow().is_empty() && portal.activity.get().elapsed() > idle)
            {
                server_stop.store(true, Ordering::Relaxed);
                if let Some(app) = portal.app.upgrade() {
                    app.quit();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        Ok(portal)
    })();
    *slot.borrow_mut() = Some(setup?);
    // GtkApplication's default startup may inspect the desktop portal. During
    // cold activation that frontend is loading our proxy, so dispatch its
    // property/introspection requests before entering GTK's synchronous startup.
    let startup = glib::MainLoop::new(None, false);
    let completed = startup.clone();
    slot.borrow().as_ref().unwrap().bus.call(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "StartServiceByName",
        Some(&("org.freedesktop.portal.Desktop", 0u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        move |result| {
            if result.is_err() {
                trace("frontend activation unavailable; serving direct backend requests");
            }
            completed.quit();
        },
    );
    startup.run();
    app.connect_startup(|_| trace("application startup complete"));
    app.connect_activate(|_| {});
    app.run_with_args(&[NAME]);
    stop.store(true, Ordering::Relaxed);
    if let Some(portal) = slot.borrow_mut().take() {
        portal.shutdown();
    }
    if let Some(path) = instance {
        crate::backend::remove_instance(&path);
    }
    Ok(())
}
