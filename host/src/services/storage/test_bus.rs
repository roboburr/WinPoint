//! A D-Bus test fake for batch D's battery, wellbeing and storage tests (notes/claude/settings-parity.md §7): objects
//! with their real introspection data on the private session bus of tests/rust-sandbox/run-tests.sh, reached by the
//! fake's unique name, so tests run side by side and never own a well-known name (as display/fake.rs). Every method
//! call is recorded with its arguments and whether the caller allowed interactive authorization
//! (system/common.rs `call_interactive`); GDBus refuses a call whose signature differs from the interface's.
use crate::services::system::common::variant_json;
use glib::variant::ToVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// One method call the fake got.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub path: String,
    pub method: String,
    pub args: Value,
    pub interactive: bool,
}

/// An object: its path and one interface (`<interface name=...>...</interface>`).
pub(crate) struct Object {
    pub path: String,
    pub xml: String,
}

/// What a method answers: its out value (a tuple, or None) or a D-Bus error (name, message).
pub(crate) type Answer = Result<Option<glib::Variant>, (String, String)>;
/// The fake's methods: (object path, method, arguments) → the answer; it may change properties.
pub(crate) type Handler = Arc<dyn Fn(&Props, &str, &str, &glib::Variant) -> Answer + Send + Sync>;

/// The properties of every object, by (path, name).
#[derive(Clone, Default)]
pub(crate) struct Props(Arc<Mutex<HashMap<(String, String), glib::Variant>>>);

impl Props {
    pub(crate) fn set(&self, path: &str, name: &str, value: glib::Variant) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((path.to_owned(), name.to_owned()), value);
    }
    pub(crate) fn get(&self, path: &str, name: &str) -> Option<glib::Variant> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(path.to_owned(), name.to_owned()))
            .cloned()
    }
}

/// A running fake.
pub(crate) struct TestBus {
    pub name: String,
    pub props: Props,
    calls: Arc<Mutex<Vec<Call>>>,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestBus {
    /// The fake on the private bus with these objects; None outside the sandbox (the caller skips).
    pub(crate) fn start(objects: Vec<Object>, props: Props, handler: Handler) -> Option<TestBus> {
        let address = crate::services::display::fake::private_bus_address()?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (thread_calls, thread_props, thread_stop) =
            (calls.clone(), props.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    gio::Cancellable::NONE,
                )
                .expect("private bus");
                let mut ids = Vec::new();
                for object in &objects {
                    let node = gio::DBusNodeInfo::for_xml(&format!("<node>{}</node>", object.xml))
                        .expect("xml");
                    let iface = node.interfaces().first().cloned().expect("an interface");
                    let (calls, handler, p1, p2) = (
                        thread_calls.clone(),
                        handler.clone(),
                        thread_props.clone(),
                        thread_props.clone(),
                    );
                    let id = connection
                        .register_object(
                            &object.path,
                            &iface,
                            move |_, _, path, _, method, params, invocation| {
                                let interactive = invocation.message().flags().contains(
                                    gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                                );
                                calls.lock().unwrap_or_else(|e| e.into_inner()).push(Call {
                                    path: path.to_owned(),
                                    method: method.to_owned(),
                                    args: variant_json(&params),
                                    interactive,
                                });
                                match handler(&p1, path, method, &params) {
                                    Ok(value) => invocation.return_value(value.as_ref()),
                                    Err((name, message)) => {
                                        invocation.return_dbus_error(&name, &message)
                                    }
                                }
                            },
                            move |_, _, path, _, property| {
                                p2.get(path, property).unwrap_or_else(|| false.to_variant())
                            },
                            |_, _, _, _, _, _| false,
                        )
                        .expect("register");
                    ids.push(id);
                }
                ready
                    .send((
                        connection.unique_name().expect("name").to_string(),
                        context.clone(),
                    ))
                    .expect("ready");
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                for id in ids {
                    let _ = connection.unregister_object(id);
                }
                let _ = connection.close_sync(gio::Cancellable::NONE);
            });
        });
        let (name, context) = started.recv().ok()?;
        Some(TestBus {
            name,
            props,
            calls,
            stop,
            context,
            thread: Some(thread),
        })
    }

    /// The calls of one method so far.
    pub(crate) fn calls_of(&self, method: &str) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|c| c.method == method)
            .cloned()
            .collect()
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The sandbox's private session bus as a client (the code under test talks to the fake over it).
pub(crate) fn client() -> gio::DBusConnection {
    crate::services::system::common::session().expect("the private session bus")
}

/// [`client`], or None outside the sandbox (never the live session bus).
pub(crate) fn client_or_none() -> Option<gio::DBusConnection> {
    crate::services::display::fake::private_bus_address()?;
    crate::services::system::common::session().ok()
}
