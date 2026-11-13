//! PackageKit's installed D-Bus ABI. All transactions stay on a private worker context.
use gio::prelude::*;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};
const SERVICE: &str = "org.freedesktop.PackageKit";
const PATH: &str = "/org/freedesktop/PackageKit";
const IFACE: &str = "org.freedesktop.PackageKit.Transaction";
#[derive(Default)]
struct State {
    packages: Vec<(u32, String, String)>,
    finished: Option<u32>,
    error: Option<(u32, String)>,
    method_error: Option<String>,
    percentage: Option<u32>,
    allow_cancel: bool,
    status: u32,
}
pub(super) enum Outcome {
    Done(Vec<(u32, String, String)>),
    Cancelled,
}
fn properties(value: &glib::Variant, state: &mut State) {
    if let Some(map) = value.get::<HashMap<String, glib::Variant>>() {
        for (k, v) in map {
            let v = if v.is_type(glib::VariantTy::VARIANT) {
                v.as_variant().unwrap_or(v)
            } else {
                v
            };
            match k.as_str() {
                "Percentage" => state.percentage = v.get::<u32>().filter(|n| *n <= 100),
                "AllowCancel" => state.allow_cancel = v.get::<bool>().unwrap_or(false),
                "Status" => state.status = v.get::<u32>().unwrap_or(0),
                _ => {}
            }
        }
    }
}
pub(super) fn run(
    method: &str,
    args: &glib::Variant,
    cancel: &dyn Fn() -> bool,
    update: &dyn Fn(&str, Option<u32>, bool),
) -> Result<Outcome, String> {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
                .map_err(|e| e.to_string())?;
            let reply = bus
                .call_sync(
                    Some(SERVICE),
                    PATH,
                    SERVICE,
                    "CreateTransaction",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    5000,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| e.to_string())?;
            let path = reply
                .child_value(0)
                .str()
                .filter(|s| s.starts_with('/') && s.len() <= 512)
                .ok_or("Invalid PackageKit transaction")?
                .to_string();
            let state = Rc::new(RefCell::new(State::default()));
            let observed = state.clone();
            let signals = bus.signal_subscribe(
                Some(SERVICE),
                Some(IFACE),
                None,
                Some(&path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, name, parameters| {
                    let mut s = observed.borrow_mut();
                    match name {
                        "Package" => {
                            if let Some(v) = parameters.get::<(u32, String, String)>() {
                                if s.packages.len() < 10000 {
                                    s.packages.push(v)
                                }
                            }
                        }
                        "Packages" => {
                            if let Some((values,)) =
                                parameters.get::<(Vec<(u32, String, String)>,)>()
                            {
                                let remaining = 10000usize.saturating_sub(s.packages.len());
                                s.packages.extend(values.into_iter().take(remaining))
                            }
                        }
                        "Finished" => {
                            if let Some((exit, _)) = parameters.get::<(u32, u32)>() {
                                s.finished = Some(exit)
                            }
                        }
                        "ErrorCode" => {
                            if let Some((code, detail)) = parameters.get::<(u32, String)>() {
                                s.error = Some((code, detail.chars().take(4000).collect()))
                            }
                        }
                        _ => {}
                    }
                },
            );
            let observed = state.clone();
            let property_signal = bus.signal_subscribe(
                Some(SERVICE),
                Some("org.freedesktop.DBus.Properties"),
                Some("PropertiesChanged"),
                Some(&path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, v| {
                    if v.child_value(0).str() == Some(IFACE) {
                        properties(&v.child_value(1), &mut observed.borrow_mut());
                    }
                },
            );
            bus.flush_sync(gio::Cancellable::NONE)
                .map_err(|e| e.to_string())?;
            let _ = bus.call_sync(
                Some(SERVICE),
                &path,
                IFACE,
                "SetHints",
                Some(&(vec!["interactive=true", "locale=C.UTF-8"],).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
            );
            let observed = state.clone();
            bus.call(
                Some(SERVICE),
                &path,
                IFACE,
                method,
                Some(args),
                None,
                gio::DBusCallFlags::NONE,
                i32::MAX,
                gio::Cancellable::NONE,
                move |reply| {
                    if let Err(e) = reply {
                        observed.borrow_mut().method_error = Some(if uncertain(&e) {
                            format!("outcome_unknown: {e}")
                        } else {
                            e.to_string()
                        });
                    }
                },
            );
            let begin = Instant::now();
            let mut checked = Instant::now() - Duration::from_secs(2);
            let mut cancel_sent = false;
            let mut last = None;
            let result = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if checked.elapsed() >= Duration::from_secs(1) {
                    checked = Instant::now();
                    let reply = bus.call_sync(
                        Some(SERVICE),
                        &path,
                        "org.freedesktop.DBus.Properties",
                        "GetAll",
                        Some(&(IFACE,).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        3000,
                        gio::Cancellable::NONE,
                    );
                    match reply {
                        Ok(reply) => properties(&reply.child_value(0), &mut state.borrow_mut()),
                        Err(error) => {
                            while context.pending() {
                                context.iteration(false);
                            }
                            if state.borrow().finished.is_none() {
                                state.borrow_mut().method_error = Some(format!(
                                    "outcome_unknown: PackageKit progress was lost: {error}"
                                ));
                            }
                        }
                    }
                }
                let s = state.borrow();
                let view = (s.status, s.percentage, s.allow_cancel);
                if last != Some(view) {
                    update(
                        if s.status == 31 {
                            "authenticating"
                        } else {
                            "running"
                        },
                        s.percentage,
                        s.allow_cancel,
                    );
                    last = Some(view);
                }
                if let Some(exit) = s.finished {
                    if exit == 1 {
                        break Ok(Outcome::Done(s.packages.clone()));
                    }
                    if exit == 3 {
                        break Ok(Outcome::Cancelled);
                    }
                    break Err(s
                        .error
                        .as_ref()
                        .map(|(code, message)| format!("PackageKit {code}: {message}"))
                        .unwrap_or_else(|| {
                            format!("PackageKit transaction exited with code {exit}")
                        }));
                }
                if let Some(error) = &s.method_error {
                    break Err(error.clone());
                }
                if begin.elapsed() > Duration::from_secs(2 * 3600) {
                    break Err(
                        "PackageKit transaction did not finish; its result is unknown".into(),
                    );
                }
                let can = s.allow_cancel;
                drop(s);
                if cancel() && can && !cancel_sent {
                    cancel_sent = true;
                    bus.call(
                        Some(SERVICE),
                        &path,
                        IFACE,
                        "Cancel",
                        None,
                        None,
                        gio::DBusCallFlags::NONE,
                        10000,
                        gio::Cancellable::NONE,
                        |_| {},
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            bus.signal_unsubscribe(signals);
            bus.signal_unsubscribe(property_signal);
            result
        })
        .map_err(|e| e.to_string())?
}
pub(super) fn resolve(name: &str, arch: &str) -> Result<String, String> {
    let args = ((1u64 << 2), vec![name]).to_variant();
    match run("Resolve", &args, &|| false, &|_, _, _| {})? {
        Outcome::Cancelled => Err("Package resolution cancelled".into()),
        Outcome::Done(packages) => packages
            .into_iter()
            .filter(|(info, _, _)| *info == 1)
            .find_map(|(_, id, _)| {
                let parts = id.split(';').collect::<Vec<_>>();
                if parts.len() == 4 && parts[0] == name && (arch.is_empty() || parts[2] == arch) {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| "The installed package could not be resolved".into()),
    }
}
pub(super) fn available() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else {
        return false;
    };
    ["ListNames", "ListActivatableNames"].iter().any(|method| {
        bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|v| v.child_value(0).get::<Vec<String>>())
        .is_some_and(|a| a.iter().any(|n| n == SERVICE))
    })
}

fn uncertain(e: &glib::Error) -> bool {
    e.matches(gio::IOErrorEnum::TimedOut)
        || e.matches(gio::IOErrorEnum::Closed)
        || [
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.Disconnected",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ]
        .iter()
        .any(|name| e.message().contains(name))
}
