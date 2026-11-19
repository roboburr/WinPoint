//! A D-Bus daemon fake for batch B's unit tests on tests/rust-sandbox/run-tests.sh's private session bus (never the
//! live one): objects with an interface, properties and method answers, served by a thread with its own
//! MainContext. Every method call that reaches it (Properties.Get/GetAll/Set too) is recorded with whether the
//! caller allowed interactive authorization, as tests/dbus-fakes/fakes/b0/echo.py records it for the bubblewrap
//! layer. The callers address it by its connection's unique name ([`Fake::name`]).
use glib::variant::ToVariant;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// One recorded call.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub path: String,
    pub iface: String,
    pub member: String,
    /// The arguments as GVariant text.
    pub args: String,
    pub interactive: bool,
}

/// What a method answers: its out arguments (a tuple), or a D-Bus error (name, message).
pub(crate) type Answer = Result<Option<glib::Variant>, (String, String)>;
/// A method handler: (member, arguments, the object's properties) → the answer.
pub(crate) type Handler =
    Arc<dyn Fn(&str, &glib::Variant, &Props) -> Answer + Send + Sync + 'static>;
/// The properties of all objects: (path, interface, name) → value; the handlers may change them.
pub(crate) type Props = Arc<Mutex<BTreeMap<(String, String, String), glib::Variant>>>;

/// One object of the fake: a path, an interface (introspection XML of that one interface) and its handler.
pub(crate) struct Object {
    pub path: String,
    pub xml: String,
    pub handler: Handler,
}

pub(crate) struct Fake {
    pub name: String,
    pub calls: Arc<Mutex<Vec<Call>>>,
    pub props: Props,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The private session bus of tests/rust-sandbox/run-tests.sh, None elsewhere (the test then skips).
pub(crate) fn private_bus_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

impl Fake {
    /// Start the fake with `objects` and the initial `props`; None without the private bus.
    pub(crate) fn start(
        objects: Vec<Object>,
        props: Vec<(&str, &str, &str, glib::Variant)>,
    ) -> Option<Fake> {
        let address = private_bus_address()?;
        let calls: Arc<Mutex<Vec<Call>>> = Arc::default();
        let store: Props = Arc::new(Mutex::new(
            props
                .into_iter()
                .map(|(p, i, n, v)| ((p.to_owned(), i.to_owned(), n.to_owned()), v))
                .collect(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (seen, thread_stop, thread_props) = (calls.clone(), stop.clone(), store.clone());
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
                .expect("the private session bus");
                // the record: every incoming method call, with its flags (Properties.Set never reaches a handler
                // with its message)
                let record = seen.clone();
                let filter = connection.add_filter(move |_, message, incoming| {
                    if incoming && message.message_type() == gio::DBusMessageType::MethodCall {
                        record.lock().unwrap().push(Call {
                            path: message.path().map(|p| p.to_string()).unwrap_or_default(),
                            iface: message
                                .interface()
                                .map(|p| p.to_string())
                                .unwrap_or_default(),
                            member: message.member().map(|p| p.to_string()).unwrap_or_default(),
                            args: message
                                .body()
                                .map(|b| b.print(true).to_string())
                                .unwrap_or_default(),
                            interactive: message
                                .flags()
                                .contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION),
                        });
                    }
                    Some(message.clone())
                });
                let mut ids = Vec::new();
                for object in objects {
                    let node = gio::DBusNodeInfo::for_xml(&object.xml).expect("the fake's XML");
                    // the one interface of the XML, by its name (gio 0.18's DBusInterfaceInfo has no getter)
                    let iface = object
                        .xml
                        .split("<interface name=\"")
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                        .expect("an interface name")
                        .to_owned();
                    let info = node.lookup_interface(&iface).expect("the fake's interface");
                    let (handler, props) = (object.handler.clone(), thread_props.clone());
                    let (get_props, set_props) = (thread_props.clone(), thread_props.clone());
                    let (path, iface_get, iface_set) =
                        (object.path.clone(), iface.clone(), iface.clone());
                    let (path_get, path_set) = (path.clone(), path.clone());
                    let id = connection
                        .register_object(
                            &object.path,
                            &info,
                            move |_, _, _, _, member, args, invocation| match handler(
                                member, &args, &props,
                            ) {
                                Ok(Some(out)) => invocation.return_value(Some(&out)),
                                Ok(None) => invocation.return_value(None),
                                Err((name, message)) => {
                                    invocation.return_dbus_error(&name, &message)
                                }
                            },
                            move |_, _, _, _, prop| {
                                get_props
                                    .lock()
                                    .unwrap()
                                    .get(&(path_get.clone(), iface_get.clone(), prop.to_owned()))
                                    .cloned()
                                    .unwrap_or_else(|| "".to_variant())
                            },
                            move |_, _, _, _, prop, value| {
                                set_props.lock().unwrap().insert(
                                    (path_set.clone(), iface_set.clone(), prop.to_owned()),
                                    value,
                                );
                                true
                            },
                        )
                        .expect("register the fake's object");
                    ids.push(id);
                }
                ready
                    .send((
                        connection.unique_name().unwrap().to_string(),
                        context.clone(),
                    ))
                    .unwrap();
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                for id in ids {
                    let _ = connection.unregister_object(id);
                }
                connection.remove_filter(filter);
            });
        });
        let (name, context) = started.recv().ok()?;
        Some(Fake {
            name,
            calls,
            props: store,
            stop,
            context,
            thread: Some(thread),
        })
    }

    /// The calls recorded so far of one member.
    pub(crate) fn calls_of(&self, member: &str) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.member == member)
            .cloned()
            .collect()
    }

    pub(crate) fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// A property as the fake holds it now.
    pub(crate) fn prop(&self, path: &str, iface: &str, name: &str) -> Option<glib::Variant> {
        self.props
            .lock()
            .unwrap()
            .get(&(path.to_owned(), iface.to_owned(), name.to_owned()))
            .cloned()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
