//! The GNOME extension's blur and shadow behind the app's windows (extension/frostedBridge.js; the calendar's
//! backdrop.rs generalised to the app's kind and to several windows). Once a window is mapped the app registers
//! its own pid with its kind:
//!
//!   Call("nativeBackdrop.configure", {pid, kind, transparency, highContrast, autoHide: false})
//!   -> {available, generation, serial, windows: [{pid, kind: "window", blur, shadow, focused}]}
//!
//! and later states come as Event("nativeBackdrop.changed", state). The extension blurs only the focused own
//! window: a window gets the class `w11-glass` (its Mica 90 % over the blur, native-backdrop.css) while the state
//! lists this pid with blur and the window is the active one (or the app's only window). Without the extension
//! nothing changes and the Mica stays solid.
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::cell::RefCell;

#[derive(Default)]
struct Bd {
    windows: Vec<glib::WeakRef<gtk::Window>>,
    kind: String,
    started: bool,
    transparency: bool,
    high_contrast: bool,
    /// The arguments of the last registration, so a settings change without effect does not register again.
    sent: Option<String>,
    /// Generation and serial of the last state applied (older states are dropped, as theme.js does).
    last: Option<(String, i64)>,
    /// Whether the last state lists this pid with blur.
    blur: bool,
}

thread_local! {
    static BD: RefCell<Bd> = RefCell::new(Bd { transparency: true, ..Default::default() });
}

/// Hook a window up (AppWindow does this): register when it maps, follow its focus.
pub(crate) fn add_window(window: &gtk::Window, kind: &str) {
    BD.with(|b| {
        let mut b = b.borrow_mut();
        b.windows.retain(|w| w.upgrade().is_some());
        b.windows.push(window.downgrade());
        b.kind = kind.to_owned();
    });
    window.connect_map(|_| {
        glib::idle_add_local_once(|| configure(false));
    });
    window.connect_unmap(|w| {
        w.remove_css_class("w11-glass");
        let any_mapped = BD.with(|b| b.borrow().windows.iter().filter_map(|w| w.upgrade()).any(|w| w.is_mapped()));
        if !any_mapped {
            BD.with(|b| {
                let mut b = b.borrow_mut();
                b.sent = None;
                b.last = None;
            });
        }
    });
    window.connect_is_active_notify(|_| refresh());
    start();
}

fn start() {
    if BD.with(|b| std::mem::replace(&mut b.borrow_mut().started, true)) {
        return;
    }
    if let Some(ctx) = crate::ctx::try_ctx() {
        let look = crate::theme::look();
        BD.with(|b| {
            let mut b = b.borrow_mut();
            b.transparency = look.transparency;
            b.high_contrast = look.high_contrast;
        });
        ctx.on("nativeBackdrop.changed", |_, state| {
            if state.get("reset").and_then(Value::as_bool) == Some(true) {
                BD.with(|b| {
                    let mut b = b.borrow_mut();
                    b.sent = None;
                    b.last = None;
                    b.blur = false;
                });
                refresh();
                if state["available"] == true {
                    configure(true);
                }
                return;
            }
            apply(state);
        });
        if let Some(conn) = ctx.gnome().bus().cloned() {
            // The watch lives for the process (its id only serves to stop it).
            let _ = gio::bus_watch_name_on_connection(
                &conn,
                crate::gnome::NAME,
                gio::BusNameWatcherFlags::NONE,
                |_, _, _| configure(true),
                |_, _| {
                    BD.with(|b| {
                        let mut b = b.borrow_mut();
                        b.sent = None;
                        b.last = None;
                        b.blur = false;
                    });
                    refresh();
                },
            );
        }
    }
}

/// settings changed (the kit calls this): register again when transparency or high contrast changed.
pub(crate) fn settings_changed(transparency: bool, high_contrast: bool) {
    let changed = BD.with(|b| {
        let mut b = b.borrow_mut();
        let changed = b.transparency != transparency || b.high_contrast != high_contrast;
        b.transparency = transparency;
        b.high_contrast = high_contrast;
        changed
    });
    if changed {
        configure(true);
        refresh();
    }
}

/// Register (again with `force`) once a window is mapped.
fn configure(force: bool) {
    let Some(ctx) = crate::ctx::try_ctx() else { return };
    let Some(args) = BD.with(|b| {
        let mut b = b.borrow_mut();
        let mapped = b.windows.iter().filter_map(|w| w.upgrade()).any(|w| w.is_mapped());
        if !mapped || b.kind.is_empty() {
            return None;
        }
        let args = json!({
            "pid": std::process::id(),
            "kind": b.kind,
            "transparency": b.transparency,
            "highContrast": b.high_contrast,
            "autoHide": false,
        });
        let text = args.to_string();
        if !force && b.sent.as_deref() == Some(text.as_str()) {
            return None;
        }
        b.sent = Some(text);
        b.last = None;
        Some(args)
    }) else {
        return;
    };
    ctx.gnome_call("nativeBackdrop.configure", args, |result| match result {
        Ok(state) if !state.is_null() => apply(&state),
        _ => {
            // The extension is off or refused: keep the solid Mica, register again on the next map or when the
            // bridge comes back.
            BD.with(|b| {
                let mut b = b.borrow_mut();
                b.sent = None;
                b.blur = false;
            });
            refresh();
        }
    });
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
    let blur = state["available"] == true
        && state["windows"]
            .as_array()
            .is_some_and(|ws| ws.iter().any(|w| w["pid"].as_u64() == Some(pid) && w["blur"] == true));
    BD.with(|b| b.borrow_mut().blur = blur);
    refresh();
}

/// Put `w11-glass` on the window the extension blurs.
fn refresh() {
    let (windows, on) = BD.with(|b| {
        let b = b.borrow();
        let windows: Vec<gtk::Window> = b.windows.iter().filter_map(|w| w.upgrade()).collect();
        (windows, b.blur && b.transparency && !b.high_contrast)
    });
    let mapped = windows.iter().filter(|w| w.is_mapped()).count();
    for w in &windows {
        let glass = on && w.is_mapped() && (mapped == 1 || w.is_active());
        if glass {
            w.add_css_class("w11-glass");
        } else {
            w.remove_css_class("w11-glass");
        }
    }
}
