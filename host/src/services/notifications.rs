//! Native notification store and optional freedesktop server; GNOME retains its own server.
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
    pub fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
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
        native["dnd"] = json!(self.dnd());
        native["language"] = json!(self.ctx.language());
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
