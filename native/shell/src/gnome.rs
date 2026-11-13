//! The GNOME extension (extension/panelBridge.js), over D-Bus: `Call(method, argsJSON) -> resultJSON` and its
//! `Event(event, dataJSON)` signal. The shell calls it itself, so the pid the extension checks (frosted glass,
//! desktop.ready) is the shell's own.
use gtk::gio;
use gtk::glib::{self, prelude::*};
use serde_json::{Value, json};

const NAME: &str = "io.github.win11shell.GnomeBridge";
const PATH: &str = "/io/github/win11shell/GnomeBridge";

pub struct Gnome {
    bus: Option<gio::DBusConnection>,
    _subscription: std::cell::RefCell<Option<gio::SignalSubscription>>,
}

impl Gnome {
    pub fn new() -> Gnome {
        Gnome {
            bus: gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok(),
            _subscription: Default::default(),
        }
    }

    /// A synchronous call (the extension answers at once).
    pub fn call(&self, method: &str, args: &Value) -> Result<Value, String> {
        let bus = self.bus.as_ref().ok_or("no session bus")?;
        let reply = bus
            .call_sync(
                Some(NAME),
                PATH,
                NAME,
                "Call",
                Some(&(method, args.to_string()).to_variant()),
                Some(glib::VariantTy::new("(s)").unwrap()),
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
            )
            .map_err(|e| format!("GNOME {method}: {e}"))?;
        let (text,): (String,) = reply.get().ok_or("bad reply")?;
        let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if v.get("ok") == Some(&Value::Bool(false)) {
            return Err(v["error"]["message"].as_str().unwrap_or("error").to_owned());
        }
        Ok(v)
    }

    /// Fire and forget, without waiting (input paths: clicks, keyboard handover).
    pub fn send(&self, method: &str, args: Value) {
        if let Some(bus) = &self.bus {
            bus.call(
                Some(NAME),
                PATH,
                NAME,
                "Call",
                Some(&(method, args.to_string()).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }

    /// The extension's events (windows.changed, windows.focused, notifications.*, nativeBackdrop.changed, ...).
    pub fn events(&self, on: fn(String, Value)) {
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
        *self._subscription.borrow_mut() = Some(sub);
    }

    pub fn windows(&self) -> Vec<Value> {
        self.call("windows.list", &json!({}))
            .ok()
            .and_then(|v| v.get("windows").and_then(|w| w.as_array().cloned()).or_else(|| v.as_array().cloned()))
            .unwrap_or_default()
    }
}
