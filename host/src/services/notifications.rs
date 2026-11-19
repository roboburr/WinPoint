//! Native notification store and optional freedesktop server; GNOME retains its own server.
//!
//! Settings parity, batch F (notes/claude/settings-parity.md §5.7, amendment 6): `notifications.apps` lists the apps
//! GNOME Settings' App Notifications shows and `notifications.app.set` writes one app's GNOME settings (the
//! relocatable `org.gnome.desktop.notifications.application` with its `application-id`, plus the app in
//! `application-children`), as GNOME Shell's `NotificationApplicationPolicy.store()` records an app. Every call to
//! the extension also carries `sounds` and `toasts` (settings.json), so notificationsBridge.js plays a GNOME
//! notification's sound with the WinPoint toast (amendment 4).
use super::{
    ServiceContext,
    system::common::{call, now_ms, session, variant_json},
};
use crate::files::{ApiError, ApiResult};
use gio::prelude::*;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
/// The methods (name, dangerous), as the api-methods files list them; batch F's `notifications.apps` and
/// `notifications.app.set` are in host/src/api-methods/apps.json (notes/claude/settings-parity.md §5.7;
/// services/mod.rs `method_tables`).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("notifications.app.set", true),
    ("notifications.apps", false),
    ("notifications.clear", false),
    ("notifications.consumerGone", false),
    ("notifications.consumerReady", false),
    ("notifications.delivered", false),
    ("notifications.dismiss", false),
    ("notifications.invokeAction", false),
    ("notifications.list", false),
    ("notifications.test", false),
];
const NAME: &str = "org.freedesktop.Notifications";
/// The app name 3b898e6 stored for a notification without one. The host now stores "" and the UI shows its own
/// localized label (contracts C5); an item that enters the host's own store with this value is stored as "".
/// A matcher, never translated (tests/i18n/allow-host.txt). The extension's list and events are not mapped.
const LEGACY_APP_NAME: &str = "Melding";
fn clear_legacy_app_name(item: &mut Value) {
    if let Some(item) = item.as_object_mut()
        && item.get("appName").and_then(Value::as_str) == Some(LEGACY_APP_NAME)
    {
        item.insert("appName".into(), json!(""));
    }
}
const PATH: &str = "/org/freedesktop/Notifications";
struct State {
    items: VecDeque<Value>,
    next: u32,
    expires: BTreeMap<u32, Instant>,
}
pub(crate) struct Notifications {
    ctx: Arc<ServiceContext>,
    relay: Arc<crate::gnome::EventRelay>,
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
    supported: AtomicBool,
    bus: Mutex<Option<gio::DBusConnection>>,
    main: Mutex<Option<glib::MainContext>>,
}
impl Notifications {
    pub fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let relay = crate::gnome::EventRelay::new(ctx.events.clone(), ctx.kind == "panel");
        let svc = Arc::new(Self {
            ctx,
            relay,
            state: Mutex::new(State {
                items: VecDeque::new(),
                next: 1,
                expires: BTreeMap::new(),
            }),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
            supported: AtomicBool::new(false),
            bus: Mutex::new(None),
            main: Mutex::new(None),
        });
        let weak = Arc::downgrade(&svc);
        thread::spawn(move || {
            while let Some(s) = weak.upgrade() {
                if s.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let state = s.state.lock().unwrap_or_else(|e| e.into_inner());
                let delay = state
                    .expires
                    .values()
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .min()
                    .unwrap_or(Duration::from_secs(60));
                let (state, _) = s
                    .wake
                    .wait_timeout(state, delay)
                    .unwrap_or_else(|e| e.into_inner());
                let due: Vec<u32> = state
                    .expires
                    .iter()
                    .filter(|(_, d)| **d <= Instant::now())
                    .map(|(id, _)| *id)
                    .collect();
                drop(state);
                for id in due {
                    s.close(id, 1);
                }
            }
        });
        let force = std::env::var("WIN11SHELL_NOTIFICATIONS")
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if ["own", "on", "1"].contains(&force.as_str()) {
            let weak = Arc::downgrade(&svc);
            thread::spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let Some(s) = weak.upgrade() else { return };
                    *s.main.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
                    let Ok(bus) = session() else { return };
                    let occupied = call(
                        &bus,
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "NameHasOwner",
                        Some(&(NAME,).to_variant()),
                        2000,
                    )
                    .ok()
                    .and_then(|v| v.get::<(bool,)>())
                    .is_some_and(|v| v.0);
                    if occupied {
                        return;
                    }
                    let Ok(node) = gio::DBusNodeInfo::for_xml(include_str!("notifications.xml"))
                    else {
                        return;
                    };
                    let Some(iface) = node.interfaces().first().cloned() else {
                        return;
                    };
                    let w = weak.clone();
                    let registration = bus.register_object(
                        PATH,
                        &iface,
                        move |_, sender, _, _, method, params, invocation| {
                            let Some(s) = w.upgrade() else {
                                invocation.return_dbus_error(
                                    "org.freedesktop.DBus.Error.Failed",
                                    "Service stopped",
                                );
                                return;
                            };
                            match method {
                                "GetCapabilities" => invocation.return_value(Some(
                                    &(vec![
                                        "actions",
                                        "body",
                                        "body-markup",
                                        "icon-static",
                                        "persistence",
                                    ],)
                                        .to_variant(),
                                )),
                                "GetServerInformation" => invocation.return_value(Some(
                                    &("Win11 Shell", "Win11 Shell", crate::VERSION, "1.2")
                                        .to_variant(),
                                )),
                                "CloseNotification" => {
                                    s.close(params.child_value(0).get::<u32>().unwrap_or(0), 3);
                                    invocation.return_value(None);
                                }
                                "Notify" => {
                                    let id = s.notify(&params, Some(sender));
                                    invocation.return_value(Some(&(id,).to_variant()));
                                }
                                _ => invocation.return_dbus_error(
                                    "org.freedesktop.DBus.Error.UnknownMethod",
                                    "Unknown method",
                                ),
                            }
                        },
                        |_, _, _, _, _| ().to_variant(),
                        |_, _, _, _, _, _| false,
                    );
                    let Ok(registration) = registration else {
                        return;
                    };
                    let owned = call(
                        &bus,
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        Some(&(NAME, 4u32).to_variant()),
                        2000,
                    )
                    .ok()
                    .and_then(|v| v.get::<(u32,)>())
                    .is_some_and(|v| [1, 4].contains(&v.0));
                    if !owned {
                        let _ = bus.unregister_object(registration);
                        return;
                    }
                    *s.bus.lock().unwrap_or_else(|e| e.into_inner()) = Some(bus.clone());
                    s.supported.store(true, Ordering::Relaxed);
                    s.ctx
                        .emit("notifications.status", json!({"supported":true}));
                    drop(s);
                    while let Some(s) = weak.upgrade() {
                        if s.stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        drop(s);
                        context.iteration(true);
                    }
                    if let Some(s) = weak.upgrade() {
                        s.supported.store(false, Ordering::Relaxed);
                        s.bus.lock().unwrap_or_else(|e| e.into_inner()).take();
                    }
                    let _ = call(
                        &bus,
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "ReleaseName",
                        Some(&(NAME,).to_variant()),
                        2000,
                    );
                    let _ = bus.unregister_object(registration);
                });
            });
        }
        Ok(svc)
    }
    fn dnd(&self) -> bool {
        super::system::common::truth(&self.ctx.settings()["notifications"]["dnd"])
    }
    fn emit_signal(&self, name: &str, args: glib::Variant) {
        if let Some(bus) = self.bus.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = bus.emit_signal(None, PATH, NAME, name, Some(&args));
        }
    }
    fn add(&self, mut item: Value, replaces: u32) -> Value {
        clear_legacy_app_name(&mut item);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let existing = state
            .items
            .iter()
            .position(|n| n["id"].as_u64() == Some(replaces as u64));
        let id = if replaces > 0 && existing.is_some() {
            state.items.remove(existing.unwrap());
            item["replaced"] = json!(true);
            replaces
        } else {
            let id = state.next;
            state.next = if id < 0xfffffff0 { id + 1 } else { 1 };
            id
        };
        item["id"] = json!(id);
        state.expires.remove(&id);
        if item["transient"] == true || item["expireAfter"].as_f64().is_some_and(|v| v != 0.) {
            let delay = item["expireAfter"]
                .as_f64()
                .filter(|v| *v != 0.)
                .unwrap_or(6.);
            state
                .expires
                .insert(id, Instant::now() + Duration::from_secs_f64(delay.max(0.)));
        }
        state.items.push_back(item.clone());
        while state.items.len() > 100 {
            if let Some(old) = state.items.pop_front() {
                state
                    .expires
                    .remove(&(old["id"].as_u64().unwrap_or(0) as u32));
            }
        }
        drop(state);
        self.wake.notify_all();
        self.ctx
            .emit("notifications.new", json!({"notification":item}));
        item
    }
    fn close(&self, id: u32, reason: u32) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.expires.remove(&id);
        let item = state
            .items
            .iter()
            .position(|n| n["id"].as_u64() == Some(id as u64))
            .and_then(|i| state.items.remove(i));
        drop(state);
        let Some(item) = item else { return false };
        if item["source"] == "dbus" {
            self.emit_signal("NotificationClosed", (id, reason).to_variant())
        }
        self.ctx
            .emit("notifications.closed", json!({"id":id,"reason":reason}));
        true
    }
    fn notify(&self, p: &glib::Variant, sender: Option<&str>) -> u32 {
        let string = |i| p.child_value(i).str().unwrap_or("").to_owned();
        let hints = variant_json(&p.child_value(6));
        let desktop = hints["desktop-entry"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.ends_with(".desktop") {
                    s.to_owned()
                } else {
                    format!("{s}.desktop")
                }
            });
        let info = desktop.as_ref().and_then(|id| {
            self.ctx.call("apps.list", &json!({})).ok().and_then(|a| {
                a.as_array()
                    .and_then(|a| a.iter().find(|a| a["id"].as_str() == Some(id)))
                    .cloned()
            })
        });
        let app_icon = icon_ref(&string(2));
        let mut image = hints
            .get("image-path")
            .or_else(|| hints.get("image_path"))
            .and_then(Value::as_str)
            .and_then(icon_ref);
        let raw_hints = p.child_value(6);
        for name in ["image-data", "image_data", "icon_data"] {
            if let Some(v) = variant_lookup(&raw_hints, name) {
                let v = v.as_variant().unwrap_or(v);
                if let Some(v) = image_data(&v) {
                    image = Some(v);
                    break;
                }
            }
        }
        if image.is_none() {
            if let Some(icon) = app_icon.as_ref().filter(|s| s.starts_with('/')) {
                image = Some(icon.clone());
            }
        }
        let icon = if app_icon.as_ref().is_none_or(|i| i.starts_with('/')) {
            info.as_ref()
                .and_then(|i| i["icon"].as_str())
                .map(str::to_owned)
                .or(app_icon)
                .unwrap_or("preferences-system-notifications".into())
        } else {
            app_icon.unwrap()
        };
        let actions_v = p.child_value(5).get::<Vec<String>>().unwrap_or_default();
        let mut actions = vec![];
        let mut has_default = false;
        for pair in actions_v.chunks_exact(2) {
            if pair[0] == "default" {
                has_default = true
            } else if actions.len() < 3 {
                actions.push(json!({"key":truncate(&pair[0],100),"label":truncate(if pair[1].is_empty(){&pair[0]}else{&pair[1]},60)}));
            }
        }
        let expire = p.child_value(7).get::<i32>().unwrap_or(-1);
        let urgency = if let Some(s) = hints["urgency"].as_str() {
            s.chars().next().map(|c| c as i64).unwrap_or(1)
        } else {
            super::system::common::pyint(&hints["urgency"]).unwrap_or(1)
        };
        let transient = super::system::common::truth(&hints["transient"]);
        let name = string(0);
        let name = if name.is_empty() {
            info.as_ref().and_then(|i| i["name"].as_str()).unwrap_or("")
        } else {
            &name
        };
        let name = truncate(name.trim(), 100);
        let value =
            if hints["value"].is_i64() || hints["value"].is_u64() || hints["value"].is_boolean() {
                super::system::common::pyint(&hints["value"]).filter(|n| (0..=100).contains(n))
            } else {
                None
            };
        let mut n = json!({"id":0,"appName":&name,"appIcon":icon,"summary":clean_markup(&string(3),300),"body":clean_markup(&string(4),3000),"actions":actions,"hasDefault":has_default,"time":now_ms(),"urgency":match urgency{0=>"low",2=>"critical",_=>"normal"},"image":image,"desktopId":desktop,"category":hints["category"].as_str(),"transient":transient,"resident":super::system::common::truth(&hints["resident"]),"value":value,"expireTimeout":expire,"silent":self.dnd(),"source":"dbus","sender":sender});
        if transient {
            n["expireAfter"] = json!(if expire > 0 {
                expire as f64 / 1000.
            } else {
                6.
            });
        }
        self.add(n, p.child_value(1).get::<u32>().unwrap_or(0))["id"]
            .as_u64()
            .unwrap_or(0) as u32
    }
    /// settings.json `notifications.sounds` and `notifications.showToasts` (default on), for the extension.
    fn sounds_and_toasts(&self) -> (bool, bool) {
        let settings = self.ctx.settings();
        (
            settings["notifications"]["sounds"] != false,
            settings["notifications"]["showToasts"] != false,
        )
    }
    /// What every call the extension gets carries besides its own arguments: Niet storen, the language, and
    /// (amendment 4) whether the bridge plays a GNOME notification's sound with the toast and shows toasts at all.
    /// notificationsBridge.js prefers these (`args.sounds ?? own.sounds`) over its own read of settings.json.
    fn extension_flags(&self, native: &mut Value) {
        let (sounds, toasts) = self.sounds_and_toasts();
        native["dnd"] = json!(self.dnd());
        native["sounds"] = json!(sounds);
        native["toasts"] = json!(toasts);
        native["language"] = json!(self.ctx.language());
    }
    pub fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        match method {
            "notifications.apps" => return Some(app_settings::list(self.ctx.lang())),
            "notifications.app.set" => {
                let result = app_settings::set_app(
                    &mut super::gsettings::Env::new(&Default::default(), None),
                    args,
                    self.ctx.lang(),
                    self.ctx.dry_run,
                    app_settings::installed,
                );
                if let Ok(done) = &result
                    && !self.ctx.dry_run
                {
                    self.ctx.emit(
                        "notifications.changed",
                        json!({"what": "app", "appId": done["appId"], "canonical": done["canonical"]}),
                    );
                }
                return Some(result);
            }
            _ => {}
        }
        if ![
            "notifications.list",
            "notifications.dismiss",
            "notifications.clear",
            "notifications.invokeAction",
            "notifications.test",
            "notifications.consumerReady",
            "notifications.consumerGone",
            "notifications.delivered",
        ]
        .contains(&method)
        {
            return None;
        }
        let mut native = args.clone();
        if !native.is_object() {
            native = json!({});
        }
        let id = || {
            super::system::common::pyint(&args["id"]).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "notifications.invalid",
                    crate::i18n::NO_PARAMS,
                )
            })
        };
        if [
            "notifications.dismiss",
            "notifications.invokeAction",
            "notifications.delivered",
        ]
        .contains(&method)
        {
            match id() {
                Ok(id) => native["id"] = json!(id),
                Err(e) => return Some(Err(e)),
            }
        }
        if method == "notifications.invokeAction" {
            let key = args
                .get("key")
                .filter(|v| super::system::common::truth(v))
                .cloned()
                .unwrap_or(json!("default"));
            if key.as_str().is_none_or(|k| k.chars().count() > 200) {
                return Some(Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "notifications.invalidAction",
                    crate::i18n::NO_PARAMS,
                )));
            }
            native["key"] = key;
        }
        self.extension_flags(&mut native);
        let consumer = [
            "notifications.consumerReady",
            "notifications.consumerGone",
            "notifications.delivered",
        ]
        .contains(&method);
        if consumer {
            if self.ctx.kind != "panel" {
                return Some(Err(ApiError::tr(
                    "forbidden",
                    self.ctx.lang(),
                    "notifications.panelOnly",
                    crate::i18n::NO_PARAMS,
                )));
            }
            native["pid"] = json!(std::process::id());
            if method == "notifications.consumerReady" {
                let ready = self.relay.ready() && self.ctx.subscriber_count() > 0;
                if !ready {
                    return Some(Ok(json!({"ready":false})));
                }
                native["ready"] = json!(true);
            }
        }
        // The extension's own list is passed on as it is, like its notifications.new events (gnome.rs relay): a
        // GNOME source that really is called "Melding" keeps that name in the list, the centre and its toasts alike
        // (review m2). The C5 mapping applies only to the host's own store (add()).
        match crate::gnome::call_api(method, &native) {
            Ok(Err(e)) if e.code == "unknown_method" => {}
            Ok(result) => return Some(result),
            Err(_) => {}
        }
        if consumer {
            return Some(Ok(json!({"ready":false})));
        }
        Some(match method {
            "notifications.list" => Ok(
                json!({"supported":self.supported.load(Ordering::Relaxed),"dnd":self.dnd(),"items":self.state.lock().unwrap_or_else(|e|e.into_inner()).items.iter().rev().cloned().collect::<Vec<_>>()}),
            ),
            "notifications.dismiss" => id()
                .map(|id| json!({"ok":u32::try_from(id).ok().is_some_and(|id|self.close(id,2))})),
            "notifications.clear" => {
                let ids = self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .items
                    .iter()
                    .filter_map(|n| n["id"].as_u64())
                    .collect::<Vec<_>>();
                for id in &ids {
                    self.close(*id as u32, 2);
                }
                Ok(json!({"cleared":ids.len()}))
            }
            "notifications.invokeAction" => {
                let key = args
                    .get("key")
                    .filter(|v| super::system::common::truth(v))
                    .cloned()
                    .unwrap_or(json!("default"));
                if key.as_str().is_none_or(|k| k.chars().count() > 200) {
                    Err(ApiError::tr(
                        "bad_args",
                        self.ctx.lang(),
                        "notifications.invalidAction",
                        crate::i18n::NO_PARAMS,
                    ))
                } else {
                    match id() {
                        Err(e) => Err(e),
                        Ok(id) => {
                            let item = self
                                .state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .items
                                .iter()
                                .find(|n| n["id"].as_u64() == Some(id as u64))
                                .cloned();
                            if let Some(item) = item {
                                let key = key.as_str().unwrap();
                                if item["source"] == "dbus" {
                                    self.emit_signal("ActionInvoked", (id as u32, key).to_variant())
                                }
                                self.ctx
                                    .emit("notifications.action", json!({"id":id,"key":key}));
                                if item["resident"] != true {
                                    self.close(id as u32, 2);
                                }
                                Ok(json!({}))
                            } else {
                                Err(ApiError::tr(
                                    "not_found",
                                    self.ctx.lang(),
                                    "notifications.gone",
                                    crate::i18n::NO_PARAMS,
                                ))
                            }
                        }
                    }
                }
            }
            _ => {
                let n = test_notification(self.ctx.lang(), self.dnd());
                Ok(json!({"notification":self.add(n,0)}))
            }
        })
    }
    pub fn shutdown(&self) {
        if self.ctx.kind == "panel" {
            let _ = crate::gnome::call_api(
                "notifications.consumerGone",
                &json!({"pid":std::process::id()}),
            );
        }
        self.relay.shutdown();
        self.stopped.store(true, Ordering::Relaxed);
        self.wake.notify_all();
        if let Some(context) = self.main.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            context.wakeup();
        }
    }
}
fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn icon_ref(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let value = if s.starts_with("file://") {
        gio::File::for_uri(s).path()?.to_string_lossy().into_owned()
    } else {
        s.into()
    };
    if value.starts_with('/') {
        Path::new(&value).is_file().then_some(value)
    } else {
        (value.len() <= 200
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.+-".contains(&c)))
        .then_some(value)
    }
}
fn image_data(v: &glib::Variant) -> Option<String> {
    if v.type_().as_str() != "(iiibiiay)" {
        return None;
    }
    let w = v.child_value(0).get::<i32>()?;
    let h = v.child_value(1).get::<i32>()?;
    let stride = v.child_value(2).get::<i32>()?;
    let alpha = v.child_value(3).get::<bool>()?;
    let bps = v.child_value(4).get::<i32>()?;
    let data = v.child_value(6).fixed_array::<u8>().ok()?.to_vec();
    if w <= 0
        || h <= 0
        || w > 4096
        || h > 4096
        || stride <= 0
        || bps != 8
        || data.len()
            < (stride as usize)
                .checked_mul(h as usize - 1)?
                .checked_add(w as usize * if alpha { 4 } else { 3 })?
    {
        return None;
    }
    let pb = gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from(&data),
        gdk_pixbuf::Colorspace::Rgb,
        alpha,
        bps,
        w,
        h,
        stride,
    );
    let pb = if w.max(h) > 128 {
        pb.scale_simple(
            (w * 128 / w.max(h)).max(1),
            (h * 128 / w.max(h)).max(1),
            gdk_pixbuf::InterpType::Bilinear,
        )?
    } else {
        pb
    };
    let data = pb.save_to_bufferv("png", &[]).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        glib::base64_encode(&data)
    ))
}
fn entities() -> &'static BTreeMap<String, String> {
    static TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("html-entities.json")).expect("HTML character names")
    })
}
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if c != '&' {
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let tail = &text[i + 1..];
        if tail.starts_with('#') {
            let hex = tail.starts_with("#x") || tail.starts_with("#X");
            let start = if hex { 2 } else { 1 };
            let digits = tail[start..]
                .chars()
                .take_while(|c| {
                    if hex {
                        c.is_ascii_hexdigit()
                    } else {
                        c.is_ascii_digit()
                    }
                })
                .take(if hex { usize::MAX } else { 7 })
                .collect::<String>();
            if !digits.is_empty() {
                let mut used = start + digits.len();
                if tail.as_bytes().get(used) == Some(&b';') {
                    used += 1
                }
                let n = u32::from_str_radix(&digits, if hex { 16 } else { 10 }).unwrap_or(0x110000);
                let mapped = match n {
                    0 | 0xd800..=0xdfff | 0x110000.. => 0xfffd,
                    128 => 8364,
                    130 => 8218,
                    131 => 402,
                    132 => 8222,
                    133 => 8230,
                    134 => 8224,
                    135 => 8225,
                    136 => 710,
                    137 => 8240,
                    138 => 352,
                    139 => 8249,
                    140 => 338,
                    142 => 381,
                    145 => 8216,
                    146 => 8217,
                    147 => 8220,
                    148 => 8221,
                    149 => 8226,
                    150 => 8211,
                    151 => 8212,
                    152 => 732,
                    153 => 8482,
                    154 => 353,
                    155 => 8250,
                    156 => 339,
                    158 => 382,
                    159 => 376,
                    _ => n,
                };
                if !matches!(n,1..=8|11|14..=31|127|0xfdd0..=0xfdef) && n & 0xffff < 0xfffe {
                    if let Some(c) = char::from_u32(mapped) {
                        out.push(c)
                    }
                }
                i += 1 + used;
                continue;
            }
        }
        let name = tail
            .chars()
            .take_while(|c| !matches!(c, '\t' | '\n' | '\u{c}' | ' ' | '<' | '&' | '#' | ';'))
            .take(32)
            .collect::<String>();
        let mut candidate = name.clone();
        if tail.as_bytes().get(name.len()) == Some(&b';') {
            candidate.push(';');
        }
        let mut hit = None;
        for (len, _) in candidate
            .char_indices()
            .rev()
            .map(|(i, c)| (i + c.len_utf8(), c))
        {
            if let Some(value) = entities().get(&candidate[..len]) {
                hit = Some((len, value));
                break;
            }
        }
        if let Some((len, value)) = hit {
            out.push_str(value);
            i += len + 1
        } else {
            out.push('&');
            i += 1
        }
    }
    out
}
fn clean_markup(text: &str, limit: usize) -> String {
    let text = truncate(text, limit * 4);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut clone = chars.clone();
            let mut found = false;
            while let Some(c) = clone.next() {
                if c == '>' {
                    found = true;
                    break;
                }
                tag.push(c);
                if tag.chars().count() > 500 {
                    break;
                }
            }
            if found {
                chars = clone;
                if tag
                    .trim()
                    .trim_end_matches('/')
                    .trim()
                    .eq_ignore_ascii_case("br")
                {
                    out.push('\n')
                }
                continue;
            }
        }
        out.push(c)
    }
    let out = unescape(&out);
    let out = out
        .split('\n')
        .map(|l| l.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n");
    truncate(out.trim(), limit)
}

fn variant_lookup(map: &glib::Variant, name: &str) -> Option<glib::Variant> {
    for i in 0..map.n_children() {
        let entry = map.child_value(i);
        if entry.child_value(0).str() == Some(name) {
            return Some(entry.child_value(1));
        }
    }
    None
}
/// notifications.test (Settings › Notifications). It is rendered once, in the language of the moment, and then kept
/// like every notification in the centre: a notification is content and keeps the language it was sent in.
fn test_notification(lang: crate::i18n::Lang, silent: bool) -> Value {
    json!({"id":0,"appName":"Win11 Shell","appIcon":"preferences-system-notifications","summary":crate::i18n::tr(lang, "notifications.test.summary"),"body":crate::i18n::tr(lang, "notifications.test.body"),"actions":[{"key":"ok","label":crate::i18n::tr(lang, "notifications.test.ok")}],"hasDefault":true,"time":now_ms(),"urgency":"normal","image":null,"desktopId":"win11shell-settings.desktop","category":null,"transient":false,"resident":false,"value":null,"expireTimeout":-1,"silent":silent,"source":"test"})
}
/// GNOME Settings' App Notifications (cc-notifications-panel.c, cc-app-notifications-dialog.c) and GNOME Shell's
/// record of an app (messageTray.js `NotificationApplicationPolicy`), for Instellingen › Systeem › Meldingen.
pub(crate) mod app_settings {
    use super::super::gsettings::{self, Env, KeyRef};
    use crate::files::{ApiError, ApiResult};
    use crate::i18n::{Lang, NO_PARAMS};
    use gio::prelude::*;
    use serde_json::{Map, Value, json};
    use std::collections::HashSet;

    pub(crate) const MASTER: &str = "org.gnome.desktop.notifications";
    pub(crate) const APP_SCHEMA: &str = "org.gnome.desktop.notifications.application";
    pub(crate) const APP_PREFIX: &str = "/org/gnome/desktop/notifications/application/";
    /// The per-app keys GNOME Settings shows, all booleans: Notifications, Sound, Banners (Show Banners, Show
    /// Content), Lock Screen (Show Banners, Show Content).
    pub(crate) const APP_KEYS: &[&str] = &[
        "enable",
        "enable-sound-alerts",
        "show-banners",
        "force-expanded",
        "show-in-lock-screen",
        "details-in-lock-screen",
    ];
    /// The method whose allow rows (allow/apps.rs, `Service`) cover the per-app schema.
    const METHOD: &str = "notifications.app.set";
    /// More apps than any desktop has; the answer stays bounded.
    const MAX_APPS: usize = 200;

    fn bad(lang: Lang) -> ApiError {
        ApiError::tr("bad_args", lang, "notifications.badApp", NO_PARAMS)
    }

    /// A desktop id as GIO names it, with ".desktop" (`org.gnome.Nautilus` → `org.gnome.Nautilus.desktop`); None
    /// for anything that is no desktop id.
    pub(crate) fn desktop_id(id: &str) -> Option<String> {
        let full = if id.ends_with(".desktop") {
            id.to_owned()
        } else {
            format!("{id}.desktop")
        };
        let stem = &full[..full.len() - ".desktop".len()];
        (!stem.is_empty()
            && full.len() <= 255
            && !stem.starts_with(['.', '-'])
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)))
        .then_some(full)
    }

    /// GNOME Shell's canonical id of an app (`_canonicalizeId`): the desktop id without ".desktop" in lower case,
    /// every other character than `[a-z0-9-]` a '-', never two '-' in a row. GNOME Settings writes the same path
    /// (it collapses nothing, but desktop ids never hold two such characters in a row). None when it does not fit
    /// the allow row's `{appid}` hole (`[a-z0-9-]{1,128}`).
    pub(crate) fn canonical_id(desktop_id: &str) -> Option<String> {
        let stem = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
        let mut out = String::with_capacity(stem.len());
        for c in stem.chars().flat_map(char::to_lowercase) {
            let c = if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            };
            if c == '-' && out.ends_with('-') {
                continue;
            }
            out.push(c);
        }
        (!out.is_empty() && out.len() <= 128).then_some(out)
    }

    /// The relocatable path of an app's settings.
    pub(crate) fn app_path(canonical: &str) -> String {
        format!("{APP_PREFIX}{canonical}/")
    }

    /// Whether GIO knows the desktop id (an installed app).
    pub(crate) fn installed(id: &str) -> bool {
        gio::DesktopAppInfo::new(id).is_some()
    }

    /// The values of a `notifications.app.set`: one or more of [`APP_KEYS`], each a boolean.
    pub(crate) fn parse_values(values: &Value) -> Option<Vec<(&'static str, Value)>> {
        let map = values.as_object()?;
        if map.is_empty() || map.len() > APP_KEYS.len() {
            return None;
        }
        let mut out = Vec::with_capacity(map.len());
        for (key, value) in map {
            let known = APP_KEYS.iter().find(|k| **k == key.as_str())?;
            out.push((*known, Value::Bool(value.as_bool()?)));
        }
        Some(out)
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The canonical ids GNOME Shell stored in `application-children`.
    fn children(env: &mut Env, lang: Lang) -> Vec<String> {
        gsettings::read_item_in(env, MASTER, None, "application-children", lang)
            .map(|item| strings(&item["value"]))
            .unwrap_or_default()
    }

    /// One app of the list.
    struct Entry {
        id: String,
        canonical: String,
        name: String,
        icon: Option<String>,
        uses: bool,
        listed: bool,
    }

    /// The app's name in WinPoint's language: the desktop file's `Name[nl]` (or `Name[en_GB]`, else `Name`), as
    /// GIO would give it in that locale; GIO's own name (the process's locale) when the file cannot be read.
    pub(crate) fn name_in(info: &gio::DesktopAppInfo, lang: Lang) -> String {
        // Name[en_GB] for English (Windows' en-GB), Name[nl] for Dutch
        let locale = if lang == Lang::En {
            "en_GB"
        } else {
            lang.code()
        };
        info.filename()
            .and_then(|file| {
                let kf = glib::KeyFile::new();
                kf.load_from_file(&file, glib::KeyFileFlags::KEEP_TRANSLATIONS)
                    .ok()?;
                kf.locale_string("Desktop Entry", "Name", Some(locale)).ok()
            })
            .map(|n| n.to_string())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| info.name().to_string())
    }

    fn entry(info: &gio::DesktopAppInfo, uses: bool, listed: bool, lang: Lang) -> Option<Entry> {
        let id = info.id()?.to_string();
        let canonical = canonical_id(&id)?;
        Some(Entry {
            id,
            canonical,
            name: name_in(info, lang),
            icon: info
                .icon()
                .and_then(|i| IconExt::to_string(&i))
                .map(|s| s.to_string()),
            uses,
            listed,
        })
    }

    /// `notifications.apps {}`: GNOME Settings' list, the apps whose desktop file says
    /// `X-GNOME-UsesNotifications=true` and those GNOME Shell recorded in `application-children` (when their
    /// `application-id` still names an installed app), with their six values and whether each can be written.
    pub(crate) fn list(lang: Lang) -> ApiResult {
        let mut env = Env::new(&Default::default(), None);
        list_in(
            &mut env,
            lang,
            gio::DesktopAppInfo::new,
            &gio::AppInfo::all(),
        )
    }

    /// [`list`] on `env` with the apps `all` and a desktop-id lookup (tests: a memory backend, test apps).
    pub(crate) fn list_in(
        env: &mut Env,
        lang: Lang,
        lookup: impl Fn(&str) -> Option<gio::DesktopAppInfo>,
        all: &[gio::AppInfo],
    ) -> ApiResult {
        let listed: Vec<String> = children(env, lang);
        let mut seen = HashSet::new();
        let mut apps: Vec<Entry> = Vec::new();
        for info in all {
            let Some(d) = info.downcast_ref::<gio::DesktopAppInfo>() else {
                continue;
            };
            if !d.boolean("X-GNOME-UsesNotifications") {
                continue;
            }
            if let Some(e) = entry(d, true, false, lang)
                && seen.insert(e.canonical.clone())
            {
                apps.push(e);
            }
        }
        for canonical in &listed {
            if apps.len() >= MAX_APPS {
                break;
            }
            if seen.contains(canonical) || canonical_id(canonical).as_deref() != Some(canonical) {
                continue;
            }
            let id = gsettings::read_item_in(
                env,
                APP_SCHEMA,
                Some(&app_path(canonical)),
                "application-id",
                lang,
            )
            .ok()
            .and_then(|item| item["value"].as_str().map(str::to_owned))
            .unwrap_or_default();
            let Some(info) = desktop_id(&id).and_then(|id| lookup(&id)) else {
                continue; // GNOME Settings: probably uninstalled
            };
            if let Some(mut e) = entry(&info, false, true, lang) {
                // the path GNOME Shell recorded, also when GIO's id gives another one
                e.canonical = canonical.clone();
                if seen.insert(e.canonical.clone()) {
                    apps.push(e);
                }
            }
        }
        apps.truncate(MAX_APPS);
        for e in &mut apps {
            e.listed |= listed.contains(&e.canonical);
        }
        apps.sort_by_cached_key(|e| (e.name.to_lowercase(), e.id.clone()));
        let keys: Vec<KeyRef> = apps
            .iter()
            .flat_map(|e| {
                APP_KEYS.iter().map(|k| KeyRef {
                    schema: APP_SCHEMA.into(),
                    path: Some(app_path(&e.canonical)),
                    key: (*k).into(),
                })
            })
            .collect();
        let items = if keys.is_empty() {
            Vec::new()
        } else {
            gsettings::get(env, &keys, lang)?["items"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        };
        let out: Vec<Value> = apps
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut values = Map::new();
                let mut locked = Vec::new();
                for (j, key) in APP_KEYS.iter().enumerate() {
                    let item = items.get(i * APP_KEYS.len() + j).cloned().unwrap_or_default();
                    values.insert((*key).into(), item["value"].clone());
                    if item["writable"] == false {
                        locked.push(json!(key));
                    }
                }
                json!({
                    "id": e.id, "canonical": e.canonical, "path": app_path(&e.canonical), "name": e.name,
                    "icon": e.icon, "usesNotifications": e.uses, "listed": e.listed, "values": values,
                    "locked": locked,
                })
            })
            .collect();
        Ok(json!({ "apps": out }))
    }

    /// `notifications.app.set {appId, values}` (amendment 6): the app's relocatable settings with its
    /// `application-id` (gsettings::write_values' checks: the allow row `Service("notifications.app.set")`, types,
    /// lockdown), then the app in `application-children` (a Read row for `gsettings.set`: only this method adds
    /// to it, as GNOME Shell's `store()` does; a locked list is left as it is). A dry run only checks.
    pub(crate) fn set_app(
        env: &mut Env,
        args: &Value,
        lang: Lang,
        dry_run: bool,
        installed: impl Fn(&str) -> bool,
    ) -> ApiResult {
        let raw = args["appId"].as_str().ok_or_else(|| bad(lang))?;
        let id = desktop_id(raw).ok_or_else(|| bad(lang))?;
        let canonical = canonical_id(&id).ok_or_else(|| bad(lang))?;
        let values = parse_values(&args["values"]).ok_or_else(|| bad(lang))?;
        let listed = children(env, lang);
        if !installed(&id) && !listed.contains(&canonical) {
            return Err(ApiError::tr(
                "not_found",
                lang,
                "notifications.appNotFound",
                NO_PARAMS,
            ));
        }
        let path = app_path(&canonical);
        let mut writes: Vec<(&str, Value)> = vec![("application-id", json!(id))];
        writes.extend(values.iter().cloned());
        let written = gsettings::write_values_in(
            env,
            METHOD,
            APP_SCHEMA,
            Some(&path),
            &writes,
            lang,
            dry_run,
        )?;
        let mut on_list = listed.contains(&canonical);
        if !on_list && !dry_run {
            on_list = add_child(env, &canonical);
        }
        Ok(json!({
            "appId": id, "canonical": canonical, "path": path, "listed": on_list,
            "items": written["items"],
        }))
    }

    /// Add the app to `application-children` unless the key is locked (true when it is on the list).
    fn add_child(env: &mut Env, canonical: &str) -> bool {
        let Some(schema) = env.schema(MASTER) else {
            return false;
        };
        let Some(path) = schema.path() else {
            return false;
        };
        let Some(settings) = env.settings(&schema, &path) else {
            return false;
        };
        let mut list: Vec<String> = settings
            .strv("application-children")
            .iter()
            .map(|s| s.to_string())
            .collect();
        if list.iter().any(|c| c == canonical) {
            return true;
        }
        if !settings.is_writable("application-children") {
            return false;
        }
        list.push(canonical.to_owned());
        let ok = settings.set_strv("application-children", list).is_ok();
        gio::Settings::sync();
        ok
    }
}

#[cfg(test)]
mod app_settings_tests {
    use super::app_settings::*;
    use super::*;
    use crate::i18n::Lang;
    use crate::services::gsettings::{Backend, Config, Env};
    use std::sync::Mutex;

    fn memory() -> Env {
        Env::new(
            &Config {
                backend: Backend::Memory,
                ..Default::default()
            },
            None,
        )
    }

    #[test]
    fn canonical_ids_follow_gnome_shell() {
        for (id, canonical) in [
            ("org.gnome.Nautilus.desktop", "org-gnome-nautilus"),
            ("firefox_firefox.desktop", "firefox-firefox"),
            ("org.mozilla.Thunderbird", "org-mozilla-thunderbird"),
            ("snap.app--x.desktop", "snap-app-x"),
            ("Ä.desktop", "-"),
        ] {
            assert_eq!(canonical_id(id).as_deref(), Some(canonical), "{id}");
        }
        assert_eq!(canonical_id(""), None);
        assert_eq!(
            canonical_id(&format!("{}.desktop", "a".repeat(129))),
            None,
            "the {{appid}} hole"
        );
        assert_eq!(
            app_path("org-gnome-nautilus"),
            "/org/gnome/desktop/notifications/application/org-gnome-nautilus/"
        );
        for (id, full) in [
            ("org.gnome.Nautilus", Some("org.gnome.Nautilus.desktop")),
            (
                "org.gnome.Nautilus.desktop",
                Some("org.gnome.Nautilus.desktop"),
            ),
            ("google-chrome.desktop", Some("google-chrome.desktop")),
            (".desktop", None),
            ("../x.desktop", None),
            ("a/b.desktop", None),
            ("a b", None),
            ("-x", None),
        ] {
            assert_eq!(desktop_id(id).as_deref(), full, "{id}");
        }
    }

    #[test]
    fn values_are_the_six_booleans_gnome_settings_shows() {
        let v = parse_values(&json!({"enable": false, "details-in-lock-screen": true})).unwrap();
        assert!(v.contains(&("enable", json!(false))));
        assert!(v.contains(&("details-in-lock-screen", json!(true))));
        for bad in [
            json!({}),
            json!({"enable": 1}),
            json!({"application-id": "x.desktop"}),
            json!({"enable": "true"}),
            json!([true]),
            json!(null),
        ] {
            assert!(parse_values(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_first_write_records_the_app_as_gnome_shell_does() {
        let mut env = memory();
        let args = json!({"appId": "org.gnome.Nautilus", "values": {"enable-sound-alerts": false}});
        // a dry run checks everything and writes nothing
        let dry = set_app(&mut env, &args, Lang::En, true, |_| true).unwrap();
        assert_eq!(dry["canonical"], "org-gnome-nautilus");
        assert_eq!(dry["listed"], false);
        let path = app_path("org-gnome-nautilus");
        let read = |env: &mut Env, key: &str| {
            crate::services::gsettings::read_item_in(env, APP_SCHEMA, Some(&path), key, Lang::En)
                .unwrap()
        };
        assert_eq!(
            read(&mut env, "enable-sound-alerts")["value"],
            true,
            "the dry run wrote nothing"
        );
        assert_eq!(read(&mut env, "application-id")["value"], "");
        // the real write: the child with its application-id, then application-children
        let done = set_app(&mut env, &args, Lang::En, false, |_| true).unwrap();
        assert_eq!(done["listed"], true);
        assert_eq!(done["path"], path.as_str());
        assert_eq!(read(&mut env, "enable-sound-alerts")["value"], false);
        assert_eq!(
            read(&mut env, "application-id")["value"],
            "org.gnome.Nautilus.desktop"
        );
        let children = crate::services::gsettings::read_item_in(
            &mut env,
            MASTER,
            None,
            "application-children",
            Lang::En,
        )
        .unwrap();
        assert_eq!(children["value"], json!(["org-gnome-nautilus"]));
        // once listed, a second write leaves the list alone
        set_app(
            &mut env,
            &json!({"appId": "org.gnome.Nautilus.desktop", "values": {"enable": false}}),
            Lang::En,
            false,
            |_| false,
        )
        .unwrap();
        let children = crate::services::gsettings::read_item_in(
            &mut env,
            MASTER,
            None,
            "application-children",
            Lang::En,
        )
        .unwrap();
        assert_eq!(children["value"], json!(["org-gnome-nautilus"]));
        assert_eq!(read(&mut env, "enable")["value"], false);
        // the list names it now
        let listed = list_in(&mut env, Lang::En, |_| None, &[]).unwrap();
        assert_eq!(
            listed["apps"],
            json!([]),
            "an application-id no installed app has is left out, as GNOME Settings"
        );
    }

    #[test]
    fn refusals_name_the_reason_in_both_languages() {
        let mut env = memory();
        for (lang, bad, missing) in [
            (
                Lang::En,
                "Invalid notification setting.",
                "This app wasn’t found.",
            ),
            (
                Lang::Nl,
                "Ongeldige meldingsinstelling.",
                "Deze app is niet gevonden.",
            ),
        ] {
            for args in [
                json!({"appId": "a b", "values": {"enable": true}}),
                json!({"appId": "x.desktop", "values": {}}),
                json!({"appId": "x.desktop", "values": {"application-id": "y.desktop"}}),
                json!({"values": {"enable": true}}),
            ] {
                let e = set_app(&mut env, &args, lang, false, |_| true).unwrap_err();
                assert_eq!(
                    (e.code.as_str(), e.message.as_str()),
                    ("bad_args", bad),
                    "{args}"
                );
            }
            let e = set_app(
                &mut env,
                &json!({"appId": "nothing.here", "values": {"enable": true}}),
                lang,
                false,
                |_| false,
            )
            .unwrap_err();
            assert_eq!(
                (e.code.as_str(), e.message.as_str()),
                ("not_found", missing)
            );
        }
        let children = crate::services::gsettings::read_item_in(
            &mut env,
            MASTER,
            None,
            "application-children",
            Lang::En,
        )
        .unwrap();
        assert_eq!(children["value"], json!([]), "nothing refused was recorded");
    }

    #[test]
    fn the_extension_hears_the_sound_and_toast_settings() {
        let lc = crate::services::system::common::LangContext::new("en", "desktop");
        let svc = Notifications::new(lc.ctx.clone()).unwrap();
        let settings = lc.ctx.config_dir.join("settings.json");
        let flags = |file: Value| {
            std::fs::write(&settings, file.to_string()).unwrap();
            let mut native = json!({"id": 7});
            svc.extension_flags(&mut native);
            native
        };
        // the defaults (settings-defaults.json): both on
        let native = flags(json!({"language": "en"}));
        assert_eq!(svc.sounds_and_toasts(), (true, true), "both default on");
        assert_eq!(
            (&native["sounds"], &native["toasts"], &native["id"]),
            (&json!(true), &json!(true), &json!(7)),
            "the call's own arguments stay"
        );
        // "Toestaan dat meldingen geluiden afspelen" and the toasts off: the payload says so, so the bridge's
        // `args.sounds ?? own.sounds` plays nothing (amendment 63)
        let native = flags(
            json!({"language": "en", "notifications": {"sounds": false, "showToasts": false}}),
        );
        assert_eq!(svc.sounds_and_toasts(), (false, false));
        assert_eq!(
            (&native["sounds"], &native["toasts"], &native["language"]),
            (&json!(false), &json!(false), &json!("en"))
        );
        // each key on its own
        let native = flags(json!({"language": "nl", "notifications": {"sounds": false}}));
        assert_eq!(
            (&native["sounds"], &native["toasts"], &native["language"]),
            (&json!(false), &json!(true), &json!("nl"))
        );
        let native = flags(json!({"language": "en", "notifications": {"showToasts": false}}));
        assert_eq!(
            (&native["sounds"], &native["toasts"]),
            (&json!(true), &json!(false))
        );
        assert_eq!(native["dnd"], json!(svc.dnd()));
        svc.shutdown();
    }

    /// A desktop file in the private XDG_DATA_HOME, once GIO's app index has it.
    fn install_app(id: &str, body: &str) -> Option<std::path::PathBuf> {
        let dir = glib::user_data_dir().join("applications");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(id);
        std::fs::write(&path, body).ok()?;
        for _ in 0..150 {
            if gio::DesktopAppInfo::new(id).is_some() {
                return Some(path);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// tests/rust-sandbox: keyfile GSettings and a private HOME (never the live session).
    fn sandbox() -> bool {
        crate::services::privacy::tests::sandbox().is_some()
    }

    #[test]
    fn the_writes_of_notifications_app_set_reach_gsettings_changed() {
        // the page reads the app's keys through gsettings.get and follows gsettings.changed, while
        // notifications.app.set writes them through settings objects of its own: the worker must hear them
        if !sandbox() {
            eprintln!("skipped: not in tests/rust-sandbox");
            return;
        }
        let root = std::env::temp_dir().join(format!("notif-follow-{}", crate::events::nonce()));
        std::fs::create_dir_all(&root).unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        events.on(
            "gsettings.changed",
            Arc::new(move |data: &Value| record.lock().unwrap().push(data.clone())),
        );
        let ctx = crate::services::ServiceContext::new(
            events,
            root.join("config"),
            root.join("cache"),
            root.clone(),
            "settings".into(),
            false,
        )
        .unwrap();
        let service = crate::services::gsettings::GSettings::new(ctx.clone()).unwrap();
        let path = app_path("org-example-follow");
        service
            .call(
                "gsettings.get",
                &json!({"keys": [{"schema": APP_SCHEMA, "path": path, "key": "enable-sound-alerts"}]}),
            )
            .unwrap()
            .unwrap();
        set_app(
            &mut Env::new(&Default::default(), None),
            &json!({"appId": "org.example.Follow", "values": {"enable-sound-alerts": false}}),
            Lang::En,
            false,
            |_| true,
        )
        .unwrap();
        let heard = || {
            seen.lock().unwrap().iter().any(|d| {
                d["path"] == path.as_str()
                    && d["key"] == "enable-sound-alerts"
                    && d["item"]["value"] == false
                    && d["item"]["method"] == "notifications.app.set"
            })
        };
        for _ in 0..50 {
            if heard() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let ok = heard();
        // put the sandbox back
        let child = gio::Settings::with_path(APP_SCHEMA, &path);
        for key in ["enable-sound-alerts", "application-id"] {
            child.reset(key);
        }
        let master = gio::Settings::new(MASTER);
        let rest: Vec<String> = master
            .strv("application-children")
            .iter()
            .map(|s| s.to_string())
            .filter(|c| c != "org-example-follow")
            .collect();
        master.set_strv("application-children", rest).unwrap();
        gio::Settings::sync();
        service.shutdown();
        let _ = std::fs::remove_dir_all(&root);
        assert!(ok, "{:?}", seen.lock().unwrap());
    }

    #[test]
    fn the_service_lists_and_writes_an_app_in_the_sandbox() {
        if !sandbox() {
            eprintln!("skipped: not in tests/rust-sandbox");
            return;
        }
        let id = "org.example.Pinger.desktop";
        let file = install_app(
            id,
            "[Desktop Entry]\nType=Application\nName=Pinger\nName[nl]=Pieper\nExec=true\nIcon=mail-send\nX-GNOME-UsesNotifications=true\n",
        )
        .expect("GIO did not see the test app");
        let lc = crate::services::system::common::LangContext::new("nl", "settings");
        let svc = Notifications::new(lc.ctx.clone()).unwrap();
        let apps = svc.call("notifications.apps", &json!({})).unwrap().unwrap();
        let mine = apps["apps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .cloned()
            .expect("the test app is listed");
        assert_eq!(mine["canonical"], "org-example-pinger");
        assert_eq!(mine["usesNotifications"], true);
        assert_eq!(mine["values"]["enable"], true);
        assert_eq!(mine["values"]["force-expanded"], false);
        assert_eq!(mine["icon"], "mail-send");
        assert_eq!(mine["name"], "Pieper", "the name in WinPoint's language");
        let info = gio::DesktopAppInfo::new(id).unwrap();
        assert_eq!(
            name_in(&info, Lang::En),
            "Pinger",
            "no Name[en_GB]: the untranslated one"
        );
        let done = svc
            .call(
                "notifications.app.set",
                &json!({"appId": id, "values": {"show-banners": false}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(done["listed"], true);
        let child = gio::Settings::with_path(APP_SCHEMA, &app_path("org-example-pinger"));
        assert!(!child.boolean("show-banners"));
        assert_eq!(child.string("application-id"), id);
        let master = gio::Settings::new(MASTER);
        assert!(
            master
                .strv("application-children")
                .iter()
                .any(|c| c == "org-example-pinger")
        );
        // put the sandbox back
        for key in ["show-banners", "application-id"] {
            child.reset(key);
        }
        let rest: Vec<String> = master
            .strv("application-children")
            .iter()
            .map(|s| s.to_string())
            .filter(|c| c != "org-example-pinger")
            .collect();
        master.set_strv("application-children", rest).unwrap();
        gio::Settings::sync();
        let _ = std::fs::remove_file(file);
        svc.shutdown();
    }
}

#[cfg(test)]
mod app_name_tests {
    use super::*;
    #[test]
    fn stored_melding_reads_as_an_empty_app_name_and_others_stay() {
        let mut legacy = json!({"id":1,"appName":"Melding","summary":"x"});
        clear_legacy_app_name(&mut legacy);
        assert_eq!(legacy["appName"], "");
        for name in ["Firefox", "", "Notification"] {
            let mut item = json!({"appName":name});
            clear_legacy_app_name(&mut item);
            assert_eq!(item["appName"], name);
        }
        let mut odd = json!("not an object");
        clear_legacy_app_name(&mut odd);
        assert_eq!(odd, json!("not an object"));
    }
    #[test]
    fn errors_and_the_test_notification_render_in_the_context_language() {
        // R2-A: the notifications stem in English through call() (every error here comes before the extension
        // is asked) and through the test notification's builder
        for (language, invalid, action, panel) in [
            (
                "en",
                "Invalid notification.",
                "Invalid action.",
                "Only the taskbar can take over notifications.",
            ),
            (
                "nl",
                "Ongeldige melding.",
                "Ongeldige actie.",
                "Alleen het paneel kan meldingen overnemen.",
            ),
        ] {
            let lc = crate::services::system::common::LangContext::new(language, "desktop");
            let svc = Notifications::new(lc.ctx.clone()).unwrap();
            let e = svc
                .call("notifications.dismiss", &json!({"id":"x"}))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                e.json(),
                json!({"code":"bad_args","message":invalid,"key":"notifications.invalid","params":{}})
            );
            let long = "k".repeat(201);
            let e = svc
                .call("notifications.invokeAction", &json!({"id":1,"key":long}))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                (e.message.as_str(), e.key.as_deref()),
                (action, Some("notifications.invalidAction"))
            );
            let e = svc
                .call("notifications.delivered", &json!({"id":1}))
                .unwrap()
                .unwrap_err();
            assert_eq!((e.code.as_str(), e.message.as_str()), ("forbidden", panel));
            svc.shutdown();
        }
        let en = test_notification(crate::i18n::Lang::En, false);
        assert_eq!(en["summary"], "This is what a notification looks like");
        assert_eq!(
            en["body"],
            "Notifications from your apps appear here and stay in the notification centre."
        );
        assert_eq!(en["actions"], json!([{"key":"ok","label":"Got it"}]));
        let nl = test_notification(crate::i18n::Lang::Nl, true);
        assert_eq!(nl["summary"], "Zo ziet een melding eruit");
        assert_eq!(
            nl["body"],
            "Meldingen van je apps verschijnen hier en blijven bewaard in het meldingencentrum."
        );
        assert_eq!(nl["actions"][0]["label"], "Prima");
        assert_eq!(
            (nl["appName"].as_str(), nl["silent"].as_bool()),
            (Some("Win11 Shell"), Some(true))
        );
    }
}
