//! The GNOME extension over D-Bus (native/shell/src/gnome.rs): `io.github.win11shell.GnomeBridge`
//! `Call(method, argsJSON) -> resultJSON` and its `Event(name, dataJSON)` signal. The app calls it itself, so the
//! pid the extension checks (blur, window actions) is the app's own. Without the extension every call fails with
//! code "unavailable" and no events come.
use crate::backend::CallError;
use gtk::gio;
use gtk::glib::{self, prelude::*};
use serde_json::Value;

pub const NAME: &str = "io.github.win11shell.GnomeBridge";
pub const PATH: &str = "/io/github/win11shell/GnomeBridge";

pub struct Gnome {
    bus: Option<gio::DBusConnection>,
    subscription: std::cell::RefCell<Option<gio::SignalSubscription>>,
}

fn en() -> bool {
    crate::i18n::lang() == "en"
}

fn unavailable(method: &str, detail: &str) -> CallError {
    let msg = if en() { "The Win11 GNOME extension is not available." } else { "De Win11 GNOME-uitbreiding is niet beschikbaar." };
    let mut e = CallError::new("unavailable", msg);
    e.data = serde_json::json!({ "method": method, "detail": detail });
    e
}

/// The extension's answer as a result: `{"ok": false, "error": {...}}` is an error, anything else the value.
fn parse_reply(method: &str, text: &str) -> Result<Value, CallError> {
    let v: Value = serde_json::from_str(text).map_err(|e| unavailable(method, &e.to_string()))?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        return Err(CallError::from_json(&v["error"], "error"));
    }
    Ok(v)
}

impl Gnome {
    pub(crate) fn new() -> Gnome {
        Gnome { bus: gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok(), subscription: Default::default() }
    }

    /// The session bus, when there is one.
    pub fn bus(&self) -> Option<&gio::DBusConnection> {
        self.bus.as_ref()
    }

    /// A synchronous call (the extension answers at once; 3 s timeout). Main thread only, keep it short.
    pub fn call_sync(&self, method: &str, args: &Value) -> Result<Value, CallError> {
        let bus = self.bus.as_ref().ok_or_else(|| unavailable(method, "no session bus"))?;
        let reply = bus
            .call_sync(
                Some(NAME),
                PATH,
                NAME,
                "Call",
                Some(&(method, args.to_string()).to_variant()),
                Some(glib::VariantTy::new("(s)").expect("static variant type")),
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
                gio::Cancellable::NONE,
            )
            .map_err(|e| unavailable(method, &e.to_string()))?;
        let (text,): (String,) = reply.get().ok_or_else(|| unavailable(method, "bad reply"))?;
        parse_reply(method, &text)
    }

    /// An asynchronous call; `done` runs on the main thread.
    pub fn call(&self, method: &str, args: &Value, done: impl FnOnce(Result<Value, CallError>) + 'static) {
        let Some(bus) = self.bus.as_ref() else {
            done(Err(unavailable(method, "no session bus")));
            return;
        };
        let name = method.to_owned();
        bus.call(
            Some(NAME),
            PATH,
            NAME,
            "Call",
            Some(&(method, args.to_string()).to_variant()),
            Some(glib::VariantTy::new("(s)").expect("static variant type")),
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
            gio::Cancellable::NONE,
            move |res| {
                let result = match res {
                    Ok(v) => match v.get::<(String,)>() {
                        Some((text,)) => parse_reply(&name, &text),
                        None => Err(unavailable(&name, "bad reply")),
                    },
                    Err(e) => Err(unavailable(&name, &e.to_string())),
                };
                done(result);
            },
        );
    }

    /// Fire and forget.
    pub fn send(&self, method: &str, args: &Value) {
        if let Some(bus) = &self.bus {
            bus.call(
                Some(NAME),
                PATH,
                NAME,
                "Call",
                Some(&(method, args.to_string()).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }

    /// Deliver the extension's Event signals (windows.*, nativeBackdrop.changed, ...) to `on`.
    pub(crate) fn events(&self, on: fn(String, Value)) {
        let Some(bus) = &self.bus else { return };
        let sub = bus.subscribe_to_signal(
            None,
            Some(NAME),
            Some("Event"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some((event, data)) = signal.parameters.get::<(String, String)>() {
                    on(event, serde_json::from_str(&data).unwrap_or(Value::Null));
                }
            },
        );
        *self.subscription.borrow_mut() = Some(sub);
    }
}
