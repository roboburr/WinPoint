//! GNOME Shell bridge: calls into the extension and the event relay of its signals.
use gio::prelude::*;
use serde_json::Value;

pub const SERVICE: &str = "io.github.win11shell.GnomeBridge";
pub const OBJECT_PATH: &str = "/io/github/win11shell/GnomeBridge";
pub const INTERFACE: &str = "io.github.win11shell.GnomeBridge";

/// Call the session's GNOME extension. Connections and the bounded synchronous request are
/// created in the calling thread, so the HTTP workers do not share GTK objects.
pub fn call(method: &str, args: &Value) -> Result<Value, String> {
    let lang = crate::i18n::current_language();
    decode_reply(call_raw(method, args, lang)?, lang)
}

/// Retain structured bridge errors for services whose public APIs have error codes.
pub(crate) fn call_api(method: &str, args: &Value) -> Result<crate::files::ApiResult, String> {
    let lang = crate::i18n::current_language();
    let mut value = call_raw(method, args, lang)?;
    if value["ok"] == false && value.get("error").is_some() {
        let error = &value["error"];
        // the fallback is rendered only when the extension sent no message of its own (review n3)
        let message = error["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| crate::i18n::tr(lang, "gnome.actionFailed"));
        return Ok(Err(crate::files::ApiError::new(
            error["code"].as_str().unwrap_or("native_error"),
            &message,
        )));
    }
    if value["ok"] == true
        && let Some(result) = value.get_mut("result")
    {
        value = result.take();
    }
    Ok(Ok(value))
}

fn call_raw(method: &str, args: &Value, lang: crate::i18n::Lang) -> Result<Value, String> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| crate::i18n::trf(lang, "gnome.sessionUnreachable", &serde_json::json!({"error": error.to_string()})))?;
    let parameters = (method, args.to_string()).to_variant();
    let reply = connection
        .call_sync(
            Some(SERVICE),
            OBJECT_PATH,
            INTERFACE,
            "Call",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| format!("GNOME {method}: {error}"))?;
    let (encoded,) = reply
        .get::<(String,)>()
        .ok_or_else(|| crate::i18n::tr(lang, "gnome.invalidReply"))?;
    let value: Value =
        serde_json::from_str(&encoded).map_err(|error| crate::i18n::trf(lang, "gnome.invalidJson", &serde_json::json!({"error": error.to_string()})))?;
    Ok(value)
}

/// GNOME events that reach the event stream. Only the panel (win11-shell's backend) asks for the window list:
/// the extension pushes it, so the shell does not have to poll `windows.list`.
fn relays_event(name: &str, windows: bool) -> bool {
    name.starts_with("notifications.")
        || name == "nativeBackdrop.changed"
        || (windows && matches!(name, "windows.changed" | "windows.focused"))
}

/// A private GLib context forwards GNOME events to this host's SSE subscribers only.
/// Each independent app can listen without rebroadcasting the same event to every host.
pub(crate) struct EventRelay {
    stopped: std::sync::atomic::AtomicBool,
    ready: std::sync::atomic::AtomicBool,
    context: std::sync::Mutex<Option<glib::MainContext>>,
}
impl EventRelay {
    pub(crate) fn new(
        events: std::sync::Arc<crate::events::Events>,
        windows: bool,
    ) -> std::sync::Arc<Self> {
        use std::sync::{Arc, atomic::Ordering};
        let relay = Arc::new(Self {
            stopped: false.into(),
            ready: false.into(),
            context: std::sync::Mutex::new(None),
        });
        let weak = Arc::downgrade(&relay);
        std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let Some(relay) = weak.upgrade() else { return };
                *relay.context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
                let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                else {
                    return;
                };
                let owner_events = events.clone();
                let subscription = bus.signal_subscribe(
                    Some(SERVICE),
                    Some(INTERFACE),
                    Some("Event"),
                    Some(OBJECT_PATH),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_, _, _, _, _, params| {
                        if let Some((name, data)) = params.get::<(String, String)>()
                            && relays_event(&name, windows)
                            && let Ok(data) = serde_json::from_str(&data)
                        {
                            events.emit_local(&name, data);
                        }
                    },
                );
                let owner_subscription = bus.signal_subscribe(
                    Some("org.freedesktop.DBus"), Some("org.freedesktop.DBus"),
                    Some("NameOwnerChanged"), Some("/org/freedesktop/DBus"), Some(SERVICE),
                    gio::DBusSignalFlags::NONE,
                    move |_, _, _, _, _, params| {
                        if let Some((_, _, owner)) = params.get::<(String,String,String)>() {
                            owner_events.emit_local("nativeBackdrop.changed", serde_json::json!({"available":!owner.is_empty(),"reset":true,"windows":[]}));
                        }
                    });
                // Flush AddMatch before allowing the panel to suppress GNOME banners.
                if bus.flush_sync(gio::Cancellable::NONE).is_err() {
                    bus.signal_unsubscribe(subscription);
                    bus.signal_unsubscribe(owner_subscription);
                    return;
                }
                relay.ready.store(true, Ordering::Release);
                drop(relay);
                while let Some(relay) = weak.upgrade() {
                    if relay.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    drop(relay);
                    context.iteration(true);
                }
                if let Some(relay) = weak.upgrade() {
                    relay.ready.store(false, Ordering::Release);
                }
                bus.signal_unsubscribe(subscription);
                bus.signal_unsubscribe(owner_subscription);
            });
        });
        relay
    }
    pub(crate) fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            context.wakeup();
        }
    }
}

fn decode_reply(mut value: Value, lang: crate::i18n::Lang) -> Result<Value, String> {
    if value.get("ok").and_then(Value::as_bool) == Some(false) && value.get("error").is_some() {
        let error = value.get("error").unwrap_or(&Value::Null);
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| crate::i18n::tr(lang, "gnome.windowActionFailed")));
    }
    // windows.setIconRects returns {ok:true,count,anim}; this is a raw result, not an envelope.
    if value.get("ok").and_then(Value::as_bool) == Some(true)
        && let Some(result) = value.get_mut("result")
    {
        return Ok(result.take());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use serde_json::json;
    #[test]
    fn bridge_accepts_raw_results_without_dropping_icon_status() {
        let raw = json!({"ok":true,"count":3,"anim":false});
        assert_eq!(decode_reply(raw.clone(), Lang::Nl).unwrap(), raw);
        assert_eq!(
            decode_reply(json!({"ok":false}), Lang::Nl).unwrap(),
            json!({"ok":false})
        );
        assert_eq!(
            decode_reply(json!({"supported":true,"windows":[]}), Lang::Nl).unwrap(),
            json!({"supported":true,"windows":[]})
        );
        assert_eq!(
            decode_reply(json!({"ok":true,"result":{"x":1}}), Lang::Nl).unwrap(),
            json!({"x":1})
        );
        assert_eq!(
            decode_reply(json!({"ok":false,"error":{"message":"missing"}}), Lang::Nl).unwrap_err(),
            "missing"
        );
    }
    #[test]
    fn only_the_panel_receives_window_events() {
        for name in [
            "notifications.new",
            "notifications.closed",
            "nativeBackdrop.changed",
        ] {
            assert!(relays_event(name, false), "{name}");
            assert!(relays_event(name, true), "{name}");
        }
        for name in ["windows.changed", "windows.focused"] {
            assert!(!relays_event(name, false), "{name}");
            assert!(relays_event(name, true), "{name}");
        }
        for name in ["windows.list", "windows", "settings.changed", ""] {
            assert!(!relays_event(name, true), "{name}");
        }
    }
    /// The en render test of the `gnome` stem (round2-gate §6): the bridge's fallback text.
    #[test]
    fn bridge_errors_follow_the_settings_language() {
        for (lang, text) in [
            (Lang::Nl, "De GNOME-vensteractie is mislukt"),
            (Lang::En, "The GNOME window action failed"),
        ] {
            assert_eq!(decode_reply(json!({"ok":false,"error":{}}), lang).unwrap_err(), text);
        }
    }
}
