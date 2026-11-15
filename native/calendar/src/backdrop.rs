//! The GNOME extension's blur and shadow behind the window (extension/frostedBridge.js, D-G59, D-G64). Once the
//! window is mapped the app registers its own pid as kind "calendar":
//!
//!   io.github.win11shell.GnomeBridge  /io/github/win11shell/GnomeBridge  Call(method, argsJSON) -> resultJSON
//!   Call("nativeBackdrop.configure", {pid, kind: "calendar", transparency, highContrast, autoHide: false})
//!   -> {available, generation, serial, windows: [{pid, kind: "window", blur, shadow, focused}]}
//!
//! Later states come as the bridge's Event("nativeBackdrop.changed", stateJSON) signal. While the state lists
//! this pid with blur, the window has the class `cal-glass` and its Mica is 90 % over the blur (as the kit's apps
//! do, and the HTML apps did through 6ebf169f:vendor/ui/css/native-backdrop.css); otherwise the Mica stays
//! solid. Without the extension nothing changes. The extension blurs only the focused own window, so the state
//! flips as the focus moves.
use crate::config::Config;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::cell::RefCell;

const NAME: &str = "io.github.win11shell.GnomeBridge";
const PATH: &str = "/io/github/win11shell/GnomeBridge";

#[derive(Default)]
struct Bd {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    conn: Option<gio::DBusConnection>,
    _sub: Option<gio::SignalSubscription>,
    /// The user's choices (theme.transparency, accessibility.highContrast).
    transparency: bool,
    high_contrast: bool,
    /// The arguments of the last registration, so a settings reload without change does not register again.
    sent: Option<String>,
    /// Generation and serial of the last state applied (older states are dropped, as theme.js does).
    last: Option<(String, i64)>,
}

thread_local! {
    static BD: RefCell<Bd> = RefCell::new(Bd::default());
}

/// Hook the window up: register when it is mapped (and whenever the extension comes back).
pub fn start(window: &gtk::ApplicationWindow, cfg: &Config) {
    BD.with(|b| {
        let mut b = b.borrow_mut();
        b.window = window.downgrade();
        b.transparency = cfg.transparency;
        b.high_contrast = cfg.high_contrast;
    });
    window.connect_map(|_| {
        glib::idle_add_local_once(|| configure(false));
    });
    window.connect_unmap(|_| {
        BD.with(|b| {
            let mut b = b.borrow_mut();
            b.sent = None;
            b.last = None;
        });
        set_glass(false);
    });
    gio::bus_get(gio::BusType::Session, None::<&gio::Cancellable>, |res| {
        let Ok(conn) = res else { return };
        let sub = conn.subscribe_to_signal(
            Some(NAME),
            Some(NAME),
            Some("Event"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            |signal| {
                let Some((event, data)) = signal.parameters.get::<(String, String)>() else { return };
                if event != "nativeBackdrop.changed" {
                    return;
                }
                let state: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
                if state.get("reset").and_then(Value::as_bool) == Some(true) {
                    BD.with(|b| {
                        let mut b = b.borrow_mut();
                        b.sent = None;
                        b.last = None;
                    });
                    set_glass(false);
                    if state["available"] == true {
                        configure(true);
                    }
                    return;
                }
                apply(&state);
            },
        );
        // The watch lives for the process (its id is only needed to stop it).
        let _ = gio::bus_watch_name_on_connection(
            &conn,
            NAME,
            gio::BusNameWatcherFlags::NONE,
            |_, _, _| configure(true),
            |_, _| {
                BD.with(|b| {
                    let mut b = b.borrow_mut();
                    b.sent = None;
                    b.last = None;
                });
                set_glass(false);
            },
        );
        BD.with(|b| {
            let mut b = b.borrow_mut();
            b.conn = Some(conn);
            b._sub = Some(sub);
        });
        configure(false);
    });
}

/// settings.json changed: register again when transparency or high contrast changed.
pub fn settings(cfg: &Config) {
    let changed = BD.with(|b| {
        let mut b = b.borrow_mut();
        let changed = b.transparency != cfg.transparency || b.high_contrast != cfg.high_contrast;
        b.transparency = cfg.transparency;
        b.high_contrast = cfg.high_contrast;
        changed
    });
    if changed {
        configure(true);
    }
}

/// Register (again with `force`) once the window is mapped and the session bus is there.
fn configure(force: bool) {
    let Some((conn, args)) = BD.with(|b| {
        let mut b = b.borrow_mut();
        let window = b.window.upgrade()?;
        if !window.is_mapped() {
            return None;
        }
        let conn = b.conn.clone()?;
        let args = json!({
            "pid": std::process::id(),
            "kind": "calendar",
            "transparency": b.transparency,
            "highContrast": b.high_contrast,
            "autoHide": false,
        })
        .to_string();
        if !force && b.sent.as_deref() == Some(args.as_str()) {
            return None;
        }
        b.sent = Some(args.clone());
        b.last = None;
        Some((conn, args))
    }) else {
        return;
    };
    conn.call(
        Some(NAME),
        PATH,
        NAME,
        "Call",
        Some(&("nativeBackdrop.configure", args.as_str()).to_variant()),
        Some(glib::VariantTy::new("(s)").expect("static type")),
        gio::DBusCallFlags::NO_AUTO_START,
        3000,
        None::<&gio::Cancellable>,
        |res| {
            let state = res
                .ok()
                .and_then(|v| v.get::<(String,)>())
                .and_then(|(text,)| serde_json::from_str::<Value>(&text).ok())
                .unwrap_or(Value::Null);
            if state.get("ok") == Some(&Value::Bool(false)) || state.is_null() {
                // The extension is off or refused this window: keep the solid Mica, register again on the next
                // map or when the bridge comes back.
                BD.with(|b| b.borrow_mut().sent = None);
                set_glass(false);
                return;
            }
            apply(&state);
        },
    );
}

fn apply(state: &Value) {
    let pid = u64::from(std::process::id());
    let generation = match &state["generation"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let serial = state["serial"].as_i64().unwrap_or(0);
    let stale = BD.with(|b| {
        let mut b = b.borrow_mut();
        if let Some((g, s)) = &b.last
            && *g == generation
            && serial < *s
        {
            return true;
        }
        b.last = Some((generation, serial));
        false
    });
    if stale {
        return;
    }
    let allowed = BD.with(|b| {
        let b = b.borrow();
        b.transparency && !b.high_contrast
    });
    let blur = allowed
        && state["available"] == true
        && state["windows"]
            .as_array()
            .is_some_and(|ws| ws.iter().any(|w| w["pid"].as_u64() == Some(pid) && w["blur"] == true));
    set_glass(blur);
}

fn set_glass(on: bool) {
    let Some(window) = BD.with(|b| b.borrow().window.upgrade()) else { return };
    if on {
        window.add_css_class("cal-glass");
    } else {
        window.remove_css_class("cal-glass");
    }
}
