//! StatusNotifierItem and com.canonical.dbusmenu, without a Python runtime.
#[path = "tray/helpers.rs"]
mod helpers;
#[path = "tray/icons.rs"]
mod icons;
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use gio::prelude::*;
use glib::{Variant, variant::ToVariant};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_FDO: &str = "org.freedesktop.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const ITEM_FDO: &str = "org.freedesktop.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const DBUS: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const WATCHER_XML: &str = r#"<node><interface name="@IFACE@"><method name="RegisterStatusNotifierItem"><arg direction="in" type="s" name="service"/></method><method name="RegisterStatusNotifierHost"><arg direction="in" type="s" name="service"/></method><property name="RegisteredStatusNotifierItems" type="as" access="read"/><property name="IsStatusNotifierHostRegistered" type="b" access="read"/><property name="ProtocolVersion" type="i" access="read"/><signal name="StatusNotifierItemRegistered"><arg type="s" name="service"/></signal><signal name="StatusNotifierItemUnregistered"><arg type="s" name="service"/></signal><signal name="StatusNotifierHostRegistered"/><signal name="StatusNotifierHostUnregistered"/></interface></node>"#;
#[derive(Clone)]
struct Item {
    id: String,
    bus: String,
    path: String,
    seq: u64,
    names: HashSet<String>,
    services: HashSet<String>,
    iface: Option<String>,
    pid: Option<u32>,
    public: Option<Value>,
    menu: Option<String>,
    resolved: Option<(Option<String>, Option<Value>, Option<String>)>,
    fetching: bool,
    dirty: bool,
    attempt: usize,
    due: Option<Instant>,
    menu_due: Option<Instant>,
    refreshes: Vec<Instant>,
    waiting: bool,
}
struct State {
    role: String,
    owner: Option<String>,
    items: HashMap<String, Item>,
    seq: u64,
    hosts: HashSet<String>,
    owned: HashSet<String>,
    changed_due: Option<Instant>,
    last_payload: Value,
    known: HashMap<String, Value>,
    known_at: HashMap<String, u64>,
    known_pending: HashMap<String, Value>,
    known_due: Option<Instant>,
    catalog: Vec<CatalogApp>,
    catalog_at: Option<Instant>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            role: "none".into(),
            owner: None,
            items: HashMap::new(),
            seq: 0,
            hosts: HashSet::new(),
            owned: HashSet::new(),
            changed_due: None,
            last_payload: Value::Null,
            known: HashMap::new(),
            known_at: HashMap::new(),
            known_pending: HashMap::new(),
            known_due: None,
            catalog: Vec::new(),
            catalog_at: None,
        }
    }
}
#[derive(Clone)]
struct CatalogApp {
    id: String,
    name: String,
    icon: String,
    keys: Vec<(u8, String)>,
}
enum Work {
    Role,
    Register(String, Option<String>, bool),
    Signal(String, String, String, String, Variant),
    Props(String, String, usize, Result<Variant, glib::Error>),
    Stop,
}
pub(crate) struct Tray {
    ctx: Arc<ServiceContext>,
    weak: Weak<Self>,
    state: Mutex<State>,
    connection: Mutex<Option<gio::DBusConnection>>,
    tx: mpsc::SyncSender<Work>,
    stop: AtomicBool,
    cache: icons::Cache,
    host_name: String,
}
impl Tray {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        static INSTANCES: AtomicU64 = AtomicU64::new(0);
        let sequence = INSTANCES.fetch_add(1, Ordering::Relaxed) + 1;
        let host_name = format!(
            "org.kde.StatusNotifierHost-{}{}",
            std::process::id(),
            if sequence > 1 {
                format!("-{sequence}")
            } else {
                String::new()
            }
        );
        let (tx, rx) = mpsc::sync_channel(512);
        let svc = Arc::new_cyclic(|weak| Self {
            cache: icons::Cache::new(ctx.cache_dir.join("tray")),
            ctx,
            weak: weak.clone(),
            state: Mutex::new(State::default()),
            connection: Mutex::new(None),
            tx,
            stop: AtomicBool::new(false),
            host_name,
        });
        let mode = std::env::var("WIN11SHELL_TRAY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if ["off", "0", "no"].contains(&mode.as_str()) {
            return Ok(svc);
        }
        let weak = svc.weak.clone();
        thread::Builder::new()
            .name("tray-dbus".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let Some(svc) = weak.upgrade() else { return };
                    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                    else {
                        return;
                    };
                    *svc.connection.lock().unwrap_or_else(|e| e.into_inner()) = Some(bus.clone());
                    let mut registrations = Vec::new();
                    for iface in [WATCHER, WATCHER_FDO] {
                        let Ok(info) =
                            gio::DBusNodeInfo::for_xml(&WATCHER_XML.replace("@IFACE@", iface))
                        else {
                            continue;
                        };
                        let Some(info) = info.interfaces().first().cloned() else {
                            continue;
                        };
                        let weak = svc.weak.clone();
                        let get = svc.weak.clone();
                        if let Ok(reg) = bus.register_object(
                            WATCHER_PATH,
                            &info,
                            move |_, sender, _, _, method, args, inv| {
                                if let Some(svc) = weak.upgrade() {
                                    svc.watcher_call(sender, method, &args, inv)
                                } else {
                                    inv.return_dbus_error(
                                        "org.freedesktop.DBus.Error.Failed",
                                        "Tray stopped",
                                    )
                                }
                            },
                            move |_, _, _, _, name| {
                                get.upgrade()
                                    .map(|s| s.watcher_property(name))
                                    .unwrap_or_else(|| false.to_variant())
                            },
                            |_, _, _, _, _, _| false,
                        ) {
                            registrations.push(reg)
                        }
                    }
                    let mut subscriptions = Vec::new();
                    for (iface, path) in [
                        (DBUS, Some(DBUS_PATH)),
                        (WATCHER, Some(WATCHER_PATH)),
                        (ITEM, None),
                        (ITEM_FDO, None),
                        (PROPS, None),
                        (MENU, None),
                    ] {
                        let tx = svc.tx.clone();
                        subscriptions.push(bus.signal_subscribe(
                            None,
                            Some(iface),
                            None,
                            path,
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, sender, path, iface, member, args| {
                                let _ = tx.try_send(Work::Signal(
                                    sender.into(),
                                    path.into(),
                                    iface.into(),
                                    member.into(),
                                    args.clone(),
                                ));
                            },
                        ));
                    }
                    let _ = svc.tx.try_send(Work::Role);
                    drop(svc);
                    loop {
                        while context.pending() {
                            context.iteration(false);
                        }
                        let Some(svc) = weak.upgrade() else { break };
                        if svc.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        drop(svc);
                        thread::sleep(Duration::from_millis(5));
                    }
                    for sid in subscriptions {
                        bus.signal_unsubscribe(sid)
                    }
                    for reg in registrations {
                        let _ = bus.unregister_object(reg);
                    }
                    if let Some(svc) = weak.upgrade() {
                        let names = svc
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .owned
                            .drain()
                            .collect::<Vec<_>>();
                        for name in names {
                            let _ = bus.call_sync(
                                Some(DBUS),
                                DBUS_PATH,
                                DBUS,
                                "ReleaseName",
                                Some(&(name,).to_variant()),
                                None,
                                gio::DBusCallFlags::NO_AUTO_START,
                                1000,
                                gio::Cancellable::NONE,
                            );
                        }
                        *svc.connection.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        let weak = svc.weak.clone();
        thread::Builder::new()
            .name("tray-worker".into())
            .spawn(move || {
                loop {
                    let Some(svc) = weak.upgrade() else { break };
                    if svc.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(Work::Stop) => break,
                        Ok(Work::Role) => svc.update_role(),
                        Ok(Work::Register(service, sender, listed)) => {
                            svc.register(&service, sender.as_deref(), listed)
                        }
                        Ok(Work::Signal(sender, path, iface, member, args)) => {
                            svc.signal(&sender, &path, &iface, &member, &args)
                        }
                        Ok(Work::Props(id, iface, attempt, result)) => {
                            svc.props(&id, &iface, attempt, result)
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    svc.timers();
                }
                if let Some(svc) = weak.upgrade() {
                    svc.flush_known();
                }
            })
            .map_err(|e| e.to_string())?;
        svc.cache.prune(&HashSet::new());
        Ok(svc)
    }
    fn bus(&self) -> Option<gio::DBusConnection> {
        self.connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn dbus(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&Variant>,
        timeout: i32,
    ) -> Result<Variant, glib::Error> {
        let bus = self
            .bus()
            .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::Closed, "Bus closed"))?;
        bus.call_sync(
            Some(dest),
            path,
            iface,
            method,
            args,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            timeout,
            gio::Cancellable::NONE,
        )
    }
    fn may_own(&self) -> bool {
        match std::env::var("WIN11SHELL_TRAY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "watcher" | "own" | "on" | "1" => true,
            "host" => false,
            _ => self.ctx.kind == "panel",
        }
    }
    fn request_name(&self, name: &str) -> bool {
        match self.dbus(
            DBUS,
            DBUS_PATH,
            DBUS,
            "RequestName",
            Some(&(name, 4u32).to_variant()),
            2000,
        ) {
            Ok(r)
                if r.child_value(0)
                    .get::<u32>()
                    .is_some_and(|v| v == 1 || v == 4) =>
            {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .owned
                    .insert(name.into());
                true
            }
            _ => false,
        }
    }
    fn update_role(&self) {
        if self.bus().is_none() {
            return;
        }
        let own = self.may_own() && self.request_name(WATCHER);
        let role = if own { "watcher" } else { "host" };
        let changed = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let changed = s.role != role;
            s.role = role.into();
            s.owner = None;
            changed
        };
        if changed {
            self.ctx
                .emit("tray.status", json!({"supported":true,"role":role}));
        }
        self.changed();
        if own {
            self.request_name(WATCHER_FDO);
            self.watcher_signal("StatusNotifierHostRegistered", None);
            let ids = self
                .ordered_items(false)
                .iter()
                .map(|i| i.id.clone())
                .collect::<Vec<_>>();
            for id in ids {
                self.watcher_signal("StatusNotifierItemRegistered", Some(&(id,).to_variant()));
            }
        } else {
            self.request_name(&self.host_name);
            if let Ok(r) = self.dbus(
                DBUS,
                DBUS_PATH,
                DBUS,
                "GetNameOwner",
                Some(&(WATCHER,).to_variant()),
                2000,
            ) {
                if let Some(owner) = r.child_value(0).str() {
                    let own = self
                        .bus()
                        .and_then(|b| b.unique_name())
                        .map(|s| s.to_string());
                    if own.as_deref() != Some(owner) {
                        self.state.lock().unwrap_or_else(|e| e.into_inner()).owner =
                            Some(owner.into());
                        let _ = self.dbus(
                            WATCHER,
                            WATCHER_PATH,
                            WATCHER,
                            "RegisterStatusNotifierHost",
                            Some(&(self.host_name.as_str(),).to_variant()),
                            2000,
                        );
                        if let Ok(r) = self.dbus(
                            WATCHER,
                            WATCHER_PATH,
                            PROPS,
                            "Get",
                            Some(&(WATCHER, "RegisteredStatusNotifierItems").to_variant()),
                            2000,
                        ) {
                            if let Some(items) = r
                                .child_value(0)
                                .as_variant()
                                .and_then(|v| v.get::<Vec<String>>())
                            {
                                for item in items.into_iter().take(64) {
                                    self.register(&item, None, true)
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Ok(r) = self.dbus(DBUS, DBUS_PATH, DBUS, "ListNames", None, 2000) {
            if let Some(mut names) = r.child_value(0).get::<Vec<String>>() {
                names.retain(|s| {
                    s.starts_with("org.kde.StatusNotifierItem-")
                        || s.starts_with("org.freedesktop.StatusNotifierItem-")
                });
                names.sort();
                for name in names.into_iter().take(64) {
                    self.register(&name, None, false)
                }
            }
        }
    }
    fn ordered_items(&self, ready: bool) -> Vec<Item> {
        let mut out = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .values()
            .filter(|i| !ready || i.public.is_some())
            .cloned()
            .collect::<Vec<_>>();
        out.sort_by_key(|i| i.seq);
        out
    }
    fn public(&self) -> Vec<Value> {
        self.ordered_items(true)
            .into_iter()
            .filter_map(|i| i.public)
            .collect()
    }
    fn changed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.changed_due.is_none() {
            state.changed_due = Some(Instant::now() + Duration::from_millis(100))
        }
    }
    fn watcher_signal(&self, name: &str, params: Option<&Variant>) {
        let ifaces = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.role != "watcher" {
                return;
            }
            let mut ifaces = vec![WATCHER];
            if s.owned.contains(WATCHER_FDO) {
                ifaces.push(WATCHER_FDO)
            }
            ifaces
        };
        if let Some(bus) = self.bus() {
            for iface in ifaces {
                let _ = bus.emit_signal(None, WATCHER_PATH, iface, name, params);
            }
        }
    }
    fn watcher_property(&self, name: &str) -> Variant {
        match name {
            "RegisteredStatusNotifierItems" => self
                .ordered_items(false)
                .into_iter()
                .map(|i| i.id)
                .collect::<Vec<_>>()
                .to_variant(),
            "IsStatusNotifierHostRegistered" => true.to_variant(),
            "ProtocolVersion" => 0i32.to_variant(),
            _ => false.to_variant(),
        }
    }
    fn watcher_call(
        &self,
        sender: &str,
        method: &str,
        args: &Variant,
        inv: gio::DBusMethodInvocation,
    ) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.role != "watcher" {
            drop(s);
            inv.return_dbus_error(
                "org.freedesktop.DBus.Error.Failed",
                "Not the StatusNotifierWatcher",
            );
            return;
        }
        drop(s);
        let service = args.child_value(0).str().unwrap_or("").to_string();
        match method {
            "RegisterStatusNotifierItem" => {
                let Some((name, path)) = helpers::registration(&service, Some(sender)) else {
                    inv.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Invalid StatusNotifierItem service",
                    );
                    return;
                };
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let full = s.items.len() >= 64
                    && !s.items.values().any(|i| {
                        i.id == name.clone() + &path || i.names.contains(&name) && i.path == path
                    });
                drop(s);
                if full {
                    inv.return_dbus_error(
                        "org.freedesktop.DBus.Error.LimitsExceeded",
                        "Too many tray items",
                    );
                    return;
                }
                let _ = self
                    .tx
                    .try_send(Work::Register(service, Some(sender.into()), false));
                inv.return_value(None)
            }
            "RegisterStatusNotifierHost" => {
                if !helpers::bus_name(&service) {
                    inv.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Invalid host name",
                    );
                    return;
                }
                let added = {
                    let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    s.hosts.len() < 32 && s.hosts.insert(service)
                };
                inv.return_value(None);
                if added {
                    self.watcher_signal("StatusNotifierHostRegistered", None)
                }
            }
            _ => {
                inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method")
            }
        }
    }
    fn register(&self, service: &str, sender: Option<&str>, listed: bool) {
        let Some((name, path)) = helpers::registration(service, sender) else {
            return;
        };
        let unique = if name.starts_with(':') {
            name.clone()
        } else {
            let Ok(r) = self.dbus(
                DBUS,
                DBUS_PATH,
                DBUS,
                "GetNameOwner",
                Some(&(name.as_str(),).to_variant()),
                2000,
            ) else {
                return;
            };
            let Some(owner) = r.child_value(0).str().map(str::to_owned) else {
                return;
            };
            owner
        };
        if self
            .bus()
            .and_then(|b| b.unique_name())
            .is_some_and(|n| n == unique)
            && path == WATCHER_PATH
        {
            return;
        }
        let id = unique.clone() + &path;
        let created = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(item) = s.items.get_mut(&id) {
                if name != unique {
                    item.names.insert(name);
                }
                if listed && item.services.len() < 8 {
                    item.services.insert(service.trim().into());
                }
                fresh_due(item);
                false
            } else {
                if s.items.len() >= 64 || s.items.values().filter(|i| i.bus == unique).count() >= 16
                {
                    return;
                }
                s.seq += 1;
                let seq = s.seq;
                let names = if name != unique {
                    HashSet::from([name])
                } else {
                    HashSet::new()
                };
                let services = if listed {
                    HashSet::from([service.trim().into()])
                } else {
                    HashSet::new()
                };
                s.items.insert(
                    id.clone(),
                    Item {
                        id: id.clone(),
                        bus: unique.clone(),
                        path: path.clone(),
                        seq,
                        names,
                        services,
                        iface: None,
                        pid: None,
                        public: None,
                        menu: None,
                        resolved: None,
                        fetching: false,
                        dirty: false,
                        attempt: 0,
                        due: None,
                        menu_due: None,
                        refreshes: Vec::new(),
                        waiting: false,
                    },
                );
                true
            }
        };
        if created {
            let pid = self
                .dbus(
                    DBUS,
                    DBUS_PATH,
                    DBUS,
                    "GetConnectionUnixProcessID",
                    Some(&(unique,).to_variant()),
                    2000,
                )
                .ok()
                .and_then(|v| v.child_value(0).get::<u32>())
                .filter(|p| *p > 0);
            if let Some(item) = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .items
                .get_mut(&id)
            {
                item.pid = pid
            }
            self.watcher_signal(
                "StatusNotifierItemRegistered",
                Some(&(id.as_str(),).to_variant()),
            );
            self.fetch(&id, None, 0);
        }
    }
    fn remove(&self, id: &str) {
        let removed = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .remove(id)
            .is_some();
        if removed {
            self.watcher_signal("StatusNotifierItemUnregistered", Some(&(id,).to_variant()));
            self.changed();
        }
    }
    fn signal(&self, sender: &str, path: &str, iface: &str, member: &str, args: &Variant) {
        if iface == DBUS && member == "NameOwnerChanged" {
            let Some((name, _old, new)) = args.get::<(String, String, String)>() else {
                return;
            };
            if name == WATCHER {
                let role = self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .role
                    .clone();
                if role == "host" {
                    let _ = self.tx.try_send(Work::Role);
                }
                return;
            }
            let gone = {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                s.items
                    .values()
                    .filter(|i| {
                        i.bus == name && new.is_empty() || i.names.contains(&name) && new != i.bus
                    })
                    .map(|i| i.id.clone())
                    .collect::<Vec<_>>()
            };
            for id in gone {
                self.remove(&id)
            }
            let removed = new.is_empty()
                && self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hosts
                    .remove(&name);
            if new.is_empty() && removed {
                self.watcher_signal("StatusNotifierHostUnregistered", None)
            }
            return;
        }
        if iface == WATCHER {
            let foreign = {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                s.role == "host" && s.owner.as_deref() == Some(sender)
            };
            if !foreign {
                return;
            }
            let Some(service) = args.get::<(String,)>().map(|v| v.0) else {
                return;
            };
            match member {
                "StatusNotifierItemRegistered" => self.register(&service, None, true),
                "StatusNotifierItemUnregistered" => {
                    let gone = self
                        .ordered_items(false)
                        .into_iter()
                        .filter(|i| i.id == service.trim() || i.services.contains(service.trim()))
                        .map(|i| i.id)
                        .collect::<Vec<_>>();
                    for id in gone {
                        self.remove(&id)
                    }
                }
                _ => {}
            }
            return;
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for item in s.items.values_mut().filter(|i| i.bus == sender) {
            if path == item.path
                && ([
                    "NewTitle",
                    "NewIcon",
                    "NewAttentionIcon",
                    "NewOverlayIcon",
                    "NewToolTip",
                    "NewStatus",
                    "NewMenu",
                    "NewIconThemePath",
                ]
                .contains(&member)
                    || member == "PropertiesChanged")
            {
                fresh_due(item);
            }
            if item.menu.as_deref() == Some(path)
                && iface == MENU
                && ["LayoutUpdated", "ItemsPropertiesUpdated"].contains(&member)
                && item.menu_due.is_none()
            {
                item.menu_due = Some(Instant::now() + Duration::from_millis(100));
            }
        }
    }
    fn fetch(&self, id: &str, iface: Option<&str>, attempt: usize) {
        let item = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(item) = s.items.get_mut(id) else {
                return;
            };
            if item.fetching {
                item.dirty = true;
                return;
            }
            item.fetching = true;
            item.due = None;
            item.clone()
        };
        let iface = iface
            .map(str::to_string)
            .or(item.iface)
            .unwrap_or_else(|| ITEM.into());
        let weak = self.weak.clone();
        let id = id.to_string();
        thread::spawn(move || {
            let Some(svc) = weak.upgrade() else { return };
            let reply = svc.dbus(
                &item.bus,
                &item.path,
                PROPS,
                "GetAll",
                Some(&(iface.as_str(),).to_variant()),
                5000,
            );
            let _ = svc.tx.try_send(Work::Props(id, iface, attempt, reply));
        });
    }
    fn props(&self, id: &str, iface: &str, attempt: usize, result: Result<Variant, glib::Error>) {
        let (mut dirty, first) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(item) = s.items.get_mut(id) else {
                return;
            };
            item.fetching = false;
            let d = item.dirty;
            item.dirty = false;
            (d, item.iface.is_none())
        };
        let props = result
            .as_ref()
            .ok()
            .filter(|v| v.type_().as_str() == "(a{sv})")
            .map(|v| helpers::dict(&v.child_value(0)))
            .unwrap_or_default();
        let kind = result.as_ref().err().map(helpers::error_kind);
        if kind == Some("cancelled") {
            return;
        }
        if kind == Some("gone") {
            self.remove(id);
            return;
        }
        if first && iface == ITEM && kind != Some("timeout") && props.is_empty() {
            self.fetch(id, Some(ITEM_FDO), attempt);
            return;
        }
        if props.is_empty() {
            if !first {
                if kind == Some("noobject")
                    || result.as_ref().err().is_some_and(|e| {
                        [".UnknownMethod", ".UnknownObject", ".UnknownInterface"]
                            .iter()
                            .any(|s| helpers::error_name(e).ends_with(s))
                    })
                {
                    self.remove(id)
                } else if dirty {
                    self.schedule_refresh(id, 50, 0)
                }
                return;
            }
            if kind == Some("timeout") {
                if dirty {
                    self.schedule_refresh(id, 50, 0)
                } else if attempt < 3 {
                    self.schedule_refresh(id, [5000, 15000, 60000][attempt], attempt + 1)
                }
                return;
            }
            if attempt == 0 {
                self.schedule_refresh(id, 1000, 1)
            } else {
                self.remove(id)
            }
            return;
        }
        let item = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(item) = s.items.get_mut(id) else {
                return;
            };
            item.iface = Some(iface.into());
            item.clone()
        };
        let (public, resolved, menu) = self.resolve(&item, &props);
        let is_representative = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(current) = s.items.get_mut(id) else {
                return;
            };
            current.public = Some(public.clone());
            current.resolved = Some(resolved);
            current.menu = menu;
            s.items
                .values()
                .filter(|i| i.public.as_ref().is_some_and(|p| p["key"] == public["key"]))
                .min_by_key(|i| i.seq)
                .is_none_or(|i| i.id == id)
        };
        self.changed();
        if is_representative {
            self.note_known(&public)
        }
        if dirty {
            self.schedule_refresh(id, 50, 0)
        }
    }
    fn schedule_refresh(&self, id: &str, millis: u64, attempt: usize) {
        if let Some(item) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .get_mut(id)
        {
            if millis == 50 && attempt == 0 {
                fresh_due(item)
            } else {
                item.due = Some(Instant::now() + Duration::from_millis(millis));
                item.attempt = attempt;
                item.waiting = millis >= 5000;
            }
        }
    }
    fn resolve(
        &self,
        item: &Item,
        props: &HashMap<String, Variant>,
    ) -> (
        Value,
        (Option<String>, Option<Value>, Option<String>),
        Option<String>,
    ) {
        let text = |k: &str, limit| {
            helpers::string(props.get(k))
                .map(|s| helpers::clean(&s, limit))
                .unwrap_or_default()
        };
        let id = text("Id", 200);
        let title = text("Title", 200);
        let resolved = item.resolved.clone().unwrap_or_else(|| {
            let exe = exe_name(item.pid);
            let desktop = self.desktop_for(item.pid, &id, exe.as_deref());
            let brief = desktop.as_ref().and_then(|id| {
                self.catalog()
                    .into_iter()
                    .find(|a| a.id == *id)
                    .map(|a| json!({"name":a.name,"icon":a.icon}))
            });
            (desktop, brief, exe)
        });
        let (desktop, brief, exe) = &resolved;
        let theme = text("IconThemePath", 1024);
        let icon = self.cache.icon(
            Some(text("IconName", 200).as_str()),
            props.get("IconPixmap"),
            &theme,
        );
        let menu = helpers::string(props.get("Menu")).filter(|s| {
            !["", "/", "/NO_DBUSMENU"].contains(&s.as_str()) && helpers::object_path(s)
        });
        let tooltip = props
            .get("ToolTip")
            .filter(|v| v.type_().as_str() == "(sa(iiay)ss)")
            .map(|v| {
                (
                    helpers::markup(v.child_value(2).str().unwrap_or(""), 200),
                    helpers::markup(v.child_value(3).str().unwrap_or(""), 1000),
                )
            })
            .unwrap_or_default();
        let app_name = brief
            .as_ref()
            .and_then(|b| b["name"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| (!title.is_empty()).then(|| title.clone()))
            .or_else(|| (!id.is_empty()).then(|| id.clone()))
            .or(exe.clone())
            .unwrap_or_else(|| "App".into());
        let app_icon = brief
            .as_ref()
            .and_then(|b| b["icon"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or(icon.clone());
        let category = text("Category", 200);
        let public = json!({"id":item.id,"key":helpers::item_key(desktop.as_deref(),&id,exe.as_deref()),"sniId":id,"title":title,"appName":helpers::chars(&app_name,200),"appIcon":app_icon,"desktopId":desktop,"pid":item.pid,"category":helpers::chars(if category.is_empty(){"ApplicationStatus"}else{&category},40),"status":helpers::status(&text("Status",200)),"icon":icon,"attentionIcon":self.cache.icon(Some(text("AttentionIconName",200).as_str()),props.get("AttentionIconPixmap"),&theme),"overlayIcon":self.cache.icon(Some(text("OverlayIconName",200).as_str()),props.get("OverlayIconPixmap"),&theme),"tooltip":{"title":tooltip.0,"text":tooltip.1},"itemIsMenu":helpers::boolean(props.get("ItemIsMenu")).unwrap_or(false),"hasMenu":menu.is_some()});
        (public, resolved, menu)
    }
    fn catalog(&self) -> Vec<CatalogApp> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .catalog_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
        {
            return state.catalog.clone();
        }
        drop(state);
        let localized = self
            .ctx
            .call("apps.list", &json!({}))
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| Some((v["id"].as_str()?.to_string(), v)))
            .collect::<HashMap<_, _>>();
        let mut apps = Vec::new();
        for info in gio::AppInfo::all() {
            let Some(id) = info.id().map(|s| s.to_string()) else {
                continue;
            };
            let base = id.strip_suffix(".desktop").unwrap_or(&id);
            let priority = if info.should_show() { 0 } else { 10 };
            let icon = info
                .icon()
                .and_then(|i| {
                    if let Some(i) = i.downcast_ref::<gio::ThemedIcon>() {
                        i.names().first().map(|s| s.to_string())
                    } else if let Some(i) = i.downcast_ref::<gio::FileIcon>() {
                        i.file().path().map(|p| p.to_string_lossy().into_owned())
                    } else {
                        gio::prelude::IconExt::to_string(&i).map(|s| s.to_string())
                    }
                })
                .unwrap_or_default();
            let mut keys = vec![
                (priority + 1, base.to_lowercase()),
                (priority + 7, info.name().to_lowercase()),
            ];
            if let Some(d) = info.downcast_ref::<gio::DesktopAppInfo>() {
                if let Some(wm) = d.startup_wm_class() {
                    keys.push((priority, wm.to_lowercase()))
                }
            }
            if let Some((a, b)) = base.split_once('_') {
                if a.bytes()
                    .chain(b.bytes())
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                {
                    keys.push((priority + 2, b.into()));
                    keys.push((priority + 3, a.into()));
                }
            }
            if let Some(last) = base.rsplit('.').next().filter(|_| base.contains('.')) {
                keys.push((priority + 4, last.to_lowercase()))
            }
            use glib::translate::ToGlibPtr;
            let executable = unsafe { gio::ffi::g_app_info_get_executable(info.to_glib_none().0) };
            if !executable.is_null() {
                let exe = unsafe { std::ffi::CStr::from_ptr(executable) }.to_string_lossy();
                if let Some(name) = Path::new(exe.as_ref()).file_name() {
                    keys.push((priority + 5, name.to_string_lossy().to_lowercase()));
                }
            }

            if !icon.starts_with('/') && !icon.is_empty() {
                keys.push((priority + 6, icon.to_lowercase()))
            }
            let local = localized.get(&id);
            apps.push(CatalogApp {
                id,
                name: local
                    .and_then(|v| v["name"].as_str())
                    .unwrap_or(info.name().as_str())
                    .to_string(),
                icon: local
                    .and_then(|v| v["icon"].as_str())
                    .unwrap_or(&icon)
                    .to_string(),
                keys,
            });
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.catalog = apps.clone();
        state.catalog_at = Some(Instant::now());
        apps
    }
    fn desktop_for(&self, pid: Option<u32>, id: &str, exe: Option<&str>) -> Option<String> {
        let mut hints = pid.map(pid_hints).unwrap_or_default();
        hints.retain(|s| !interpreter(s));
        if let Some(exe) = exe {
            if !interpreter(exe) {
                hints.push(exe.into())
            }
        }
        if !helpers::generic(id) {
            hints.push(id.into())
        }
        let apps = self.catalog();
        let mut seen = HashSet::new();
        for hint in hints {
            if !seen.insert(hint.clone()) {
                continue;
            }
            let mut cands = vec![hint.clone()];
            for suffix in ["-bin", "-stable", "-browser", ".bin", "-wrapped"] {
                if hint.to_lowercase().ends_with(suffix) {
                    cands.push(hint[..hint.len() - suffix.len()].into());
                }
            }
            if let Some(last) = hint.rsplit('.').next().filter(|_| hint.contains('.')) {
                cands.push(last.into())
            }
            let cands = cands
                .iter()
                .map(|s| s.to_lowercase())
                .collect::<HashSet<_>>();
            let mut hits = apps
                .iter()
                .flat_map(|app| {
                    app.keys
                        .iter()
                        .filter(|(_, k)| cands.contains(k))
                        .map(move |(score, _)| (*score, app.id.as_str()))
                })
                .collect::<Vec<_>>();
            hits.sort();
            if let Some((_, id)) = hits.first() {
                return Some((*id).into());
            }
        }
        None
    }
    fn timers(&self) {
        let now = Instant::now();
        let (refresh, menus, changed, known) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut refresh = Vec::new();
            let mut menus = Vec::new();
            for i in s.items.values_mut() {
                if i.due.is_some_and(|t| t <= now) {
                    i.due = None;
                    refresh.push((i.id.clone(), i.attempt))
                }
                if i.menu_due.is_some_and(|t| t <= now) {
                    i.menu_due = None;
                    menus.push(i.id.clone())
                }
            }
            let changed = s.changed_due.is_some_and(|t| t <= now);
            if changed {
                s.changed_due = None
            }
            let known = s.known_due.is_some_and(|t| t <= now);
            if known {
                s.known_due = None
            }
            (refresh, menus, changed, known)
        };
        for (id, attempt) in refresh {
            self.fetch(&id, None, attempt)
        }
        for id in menus {
            self.ctx.emit("tray.menuChanged", json!({"id":id}));
        }
        if changed {
            let items = json!(self.public());
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.last_payload != items {
                s.last_payload = items.clone();
                drop(s);
                self.ctx.emit("tray.changed", json!({"items":items}));
            }
        }
        if known {
            self.flush_known();
        }
        if self.cache.prune_due() {
            let live = self
                .public()
                .iter()
                .flat_map(|p| {
                    ["icon", "attentionIcon", "overlayIcon", "appIcon"]
                        .iter()
                        .filter_map(|k| p[*k].as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .collect::<HashSet<_>>();
            self.cache.prune(&live);
        }
    }
    fn note_known(&self, pub_: &Value) {
        if self.ctx.kind != "panel" {
            return;
        }
        let now = now_ms();
        let (key, name) = (pub_["key"].as_str().unwrap_or(""), pub_["appName"].clone());
        let mut icon = pub_["appIcon"].clone();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.known.is_empty() {
            s.known = self.ctx.settings()["taskbar"]["tray"]["known"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default()
        }
        let current = s.known.get(key).cloned();
        if pub_["status"] == "attention" {
            if let Some(cur) = &current {
                if !cur["icon"].is_null() {
                    icon = cur["icon"].clone()
                }
            }
        }
        if let Some(cur) = &current {
            if (cur["name"] != name || cur["icon"] != icon)
                && now.saturating_sub(*s.known_at.get(key).unwrap_or(&0)) >= 3600000
            {
                s.known_at.insert(key.into(), now);
            } else if now.saturating_sub(cur["seen"].as_u64().unwrap_or(0)) >= 86400000 {
                if !cur["icon"].is_null() {
                    icon = cur["icon"].clone()
                }
            } else {
                return;
            }
        } else {
            s.known_at.insert(key.into(), now);
        }
        let entry = json!({"name":name,"icon":icon,"seen":now});
        s.known.insert(key.into(), entry.clone());
        s.known_pending.insert(key.into(), entry);
        s.known_due = Some(Instant::now() + Duration::from_secs(1));
    }
    fn flush_known(&self) {
        if self.ctx.kind != "panel" {
            return;
        }
        let pending = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut s.known_pending)
        };
        if pending.is_empty() {
            return;
        }
        let mut known = self.ctx.settings()["taskbar"]["tray"]["known"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        known.extend(pending);
        if known.len() > 100 {
            let mut old = known
                .iter()
                .map(|(k, v)| (v["seen"].as_u64().unwrap_or(0), k.clone()))
                .collect::<Vec<_>>();
            old.sort_by_key(|(t, _)| *t);
            let n = known.len() - 100;
            for (_, key) in old.into_iter().take(n) {
                known.remove(&key);
            }
        }
        if self
            .ctx
            .call(
                "settings.set",
                &json!({"path":"taskbar.tray.known","value":known}),
            )
            .is_ok()
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.known = known.into_iter().collect();
        }
    }
    fn find(&self, args: &Value) -> Result<Item, ApiError> {
        let Some(id) = args["id"].as_str().filter(|s| !s.is_empty()) else {
            return Err(ApiError::tr(
                "bad_args",
                self.ctx.lang(),
                "tray.invalidIcon",
                crate::i18n::NO_PARAMS,
            ));
        };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .get(id)
            .filter(|i| i.public.is_some())
            .cloned()
            .ok_or_else(|| {
                ApiError::tr(
                    "not_found",
                    self.ctx.lang(),
                    "tray.iconGone",
                    crate::i18n::NO_PARAMS,
                )
            })
    }
    fn item_call(
        &self,
        item: &Item,
        method: &str,
        args: Option<&Variant>,
    ) -> Result<bool, ApiError> {
        match self.dbus(
            &item.bus,
            &item.path,
            item.iface.as_deref().unwrap_or(ITEM),
            method,
            args,
            2000,
        ) {
            Ok(_) => Ok(true),
            Err(e) if helpers::error_kind(&e) == "unsupported" => {
                self.schedule_refresh(&item.id, 50, 0);
                Ok(false)
            }
            Err(e) => Err(self.api_error(&e, item)),
        }
    }
    fn api_error(&self, error: &glib::Error, item: &Item) -> ApiError {
        match helpers::error_kind(error) {
            "gone" | "noobject" => {
                self.remove(&item.id);
                ApiError::tr(
                    "not_found",
                    self.ctx.lang(),
                    "tray.iconGone",
                    crate::i18n::NO_PARAMS,
                )
            }
            "timeout" => ApiError::tr(
                "timeout",
                self.ctx.lang(),
                "tray.notResponding",
                crate::i18n::NO_PARAMS,
            ),
            _ => {
                let mut e = ApiError::tr(
                    "tray_failed",
                    self.ctx.lang(),
                    "tray.commandFailed",
                    crate::i18n::NO_PARAMS,
                );
                e.data = Some(json!({"error":helpers::chars(&helpers::error_name(error),200)}));
                e
            }
        }
    }
    fn menu_error(&self, error: &glib::Error, item: &Item) -> ApiError {
        if helpers::error_kind(error) == "noobject" {
            self.schedule_refresh(&item.id, 50, 0);
            ApiError::tr(
                "no_menu",
                self.ctx.lang(),
                "tray.noMenu",
                crate::i18n::NO_PARAMS,
            )
        } else {
            self.api_error(error, item)
        }
    }
    fn layout(&self, item: &Item) -> Result<(u32, Vec<Value>, Vec<i32>), ApiError> {
        let path = item.menu.as_deref().ok_or_else(|| {
            ApiError::tr(
                "no_menu",
                self.ctx.lang(),
                "tray.noMenu",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let reply = self
            .dbus(
                &item.bus,
                path,
                MENU,
                "GetLayout",
                Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
                2000,
            )
            .map_err(|e| match helpers::error_kind(&e) {
                "gone" | "noobject" | "timeout" => self.menu_error(&e, item),
                _ => {
                    if helpers::error_kind(&e) == "unsupported" {
                        self.schedule_refresh(&item.id, 50, 0)
                    }
                    ApiError::tr(
                        "menu_failed",
                        self.ctx.lang(),
                        "tray.menuFailed",
                        crate::i18n::NO_PARAMS,
                    )
                }
            })?;
        if reply.type_().as_str() != "(u(ia{sv}av))" {
            return Err(ApiError::tr(
                "menu_failed",
                self.ctx.lang(),
                "tray.menuFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        let mut parents = Vec::new();
        let items = helpers::menu(
            &reply.child_value(1),
            &|data| self.cache.menu_png(data),
            &mut parents,
        );
        Ok((
            reply.child_value(0).get::<u32>().unwrap_or(0),
            items,
            parents,
        ))
    }
    fn menu(&self, item: &Item, refresh: bool) -> ApiResult {
        let path = item.menu.as_deref().ok_or_else(|| {
            ApiError::tr(
                "no_menu",
                self.ctx.lang(),
                "tray.noMenu",
                crate::i18n::NO_PARAMS,
            )
        })?;
        if !refresh {
            if let Err(e) = self.dbus(
                &item.bus,
                path,
                MENU,
                "AboutToShow",
                Some(&(0i32,).to_variant()),
                1000,
            ) {
                if ["gone", "noobject"].contains(&helpers::error_kind(&e)) {
                    return Err(self.menu_error(&e, item));
                }
            }
        }
        let (mut revision, mut items, parents) = self.layout(item)?;
        if !refresh && !parents.is_empty() {
            if self
                .dbus(
                    &item.bus,
                    path,
                    MENU,
                    "AboutToShowGroup",
                    Some(&(parents,).to_variant()),
                    1000,
                )
                .is_ok()
            {
                let r = self.layout(item)?;
                revision = r.0;
                items = r.1;
            }
        }
        Ok(json!({"revision":revision,"items":items}))
    }
    fn menu_event(&self, item: &Item, item_id: i32, event: &str) -> ApiResult {
        let path = item.menu.as_deref().ok_or_else(|| {
            ApiError::tr(
                "no_menu",
                self.ctx.lang(),
                "tray.noMenu",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let args = (item_id, event, 0i32.to_variant(), (now_ms() / 1000) as u32).to_variant();
        if event != "clicked" {
            if let Some(bus) = self.bus() {
                bus.call(
                    Some(&item.bus),
                    path,
                    MENU,
                    "Event",
                    Some(&args),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    2000,
                    gio::Cancellable::NONE,
                    |_| {},
                );
            }
            return Ok(json!({"ok":true}));
        }
        match self.dbus(&item.bus, path, MENU, "Event", Some(&args), 2000) {
            Ok(_) => Ok(json!({"ok":true})),
            Err(e) => match helpers::error_kind(&e) {
                "gone" | "timeout" => Ok(json!({"ok":true})),
                "noobject" => Err(self.menu_error(&e, item)),
                _ => Ok(json!({"ok":false})),
            },
        }
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if ![
            "tray.list",
            "tray.activate",
            "tray.secondaryActivate",
            "tray.scroll",
            "tray.contextMenu",
            "tray.menu",
            "tray.menuEvent",
        ]
        .contains(&method)
        {
            return None;
        }
        Some((|| {
            if method == "tray.list" {
                let role = self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .role
                    .clone();
                return Ok(json!({"supported":role!="none","role":role,"items":self.public()}));
            }
            let item = self.find(args)?;
            match method {
                "tray.activate" => {
                    let point = point(args, self.ctx.lang())?;
                    if item
                        .public
                        .as_ref()
                        .is_some_and(|p| p["itemIsMenu"] == true && p["hasMenu"] == true)
                    {
                        return Ok(json!({"ok":true,"fallback":"menu"}));
                    }
                    let done = self.item_call(&item, "Activate", Some(&point.to_variant()))?;
                    Ok(json!({"ok":true,"fallback":if done{None}else{Some("menu")}}))
                }
                "tray.secondaryActivate" => Ok(
                    json!({"ok":self.item_call(&item,"SecondaryActivate",Some(&point(args, self.ctx.lang())?.to_variant()))?}),
                ),
                "tray.contextMenu" => Ok(
                    json!({"ok":self.item_call(&item,"ContextMenu",Some(&point(args, self.ctx.lang())?.to_variant()))?}),
                ),
                "tray.scroll" => {
                    let orientation = args.get("orientation").filter(|v| helpers::truth(v));
                    let orientation = match orientation {
                        None => "vertical",
                        Some(Value::String(s)) => s.as_str(),
                        _ => {
                            return Err(ApiError::tr(
                                "bad_args",
                                self.ctx.lang(),
                                "tray.badScrollDirection",
                                crate::i18n::NO_PARAMS,
                            ));
                        }
                    };
                    if !["vertical", "horizontal"].contains(&orientation) {
                        return Err(ApiError::tr(
                            "bad_args",
                            self.ctx.lang(),
                            "tray.badScrollDirection",
                            crate::i18n::NO_PARAMS,
                        ));
                    }
                    let mut delta = helpers::pyint(&args["delta"], 0)
                        .ok_or_else(|| {
                            ApiError::tr(
                                "bad_args",
                                self.ctx.lang(),
                                "tray.badScrollAmount",
                                crate::i18n::NO_PARAMS,
                            )
                        })?
                        .clamp(-12000, 12000) as i32;
                    if item.path.starts_with("/org/ayatana/NotificationItem/") {
                        let steps = if delta == 0 {
                            0
                        } else {
                            ((delta.unsigned_abs() as f64 / 120.).round_ties_even() as i32).max(1)
                        };
                        delta = if delta > 0 { -steps } else { steps }
                    }
                    Ok(
                        json!({"ok":self.item_call(&item,"Scroll",Some(&(delta,orientation).to_variant()))?}),
                    )
                }
                "tray.menu" => self.menu(&item, args["refresh"] == true),
                "tray.menuEvent" => {
                    let event = args.get("event").filter(|v| helpers::truth(v));
                    let event = match event {
                        None => "clicked",
                        Some(Value::String(s)) => s.as_str(),
                        _ => {
                            return Err(ApiError::tr(
                                "bad_args",
                                self.ctx.lang(),
                                "tray.unknownMenuAction",
                                crate::i18n::NO_PARAMS,
                            ));
                        }
                    };
                    if !["clicked", "hovered", "opened", "closed"].contains(&event) {
                        return Err(ApiError::tr(
                            "bad_args",
                            self.ctx.lang(),
                            "tray.unknownMenuAction",
                            crate::i18n::NO_PARAMS,
                        ));
                    }
                    let id = args
                        .get("itemId")
                        .filter(|v| !v.is_null())
                        .and_then(|v| helpers::pyint(v, 0))
                        .filter(|i| *i >= 0 && *i <= i32::MAX as i64)
                        .ok_or_else(|| {
                            ApiError::tr(
                                "bad_args",
                                self.ctx.lang(),
                                "tray.badMenuItem",
                                crate::i18n::NO_PARAMS,
                            )
                        })?;
                    self.menu_event(&item, id as i32, event)
                }
                _ => unreachable!(),
            }
        })())
    }
    pub(crate) fn shutdown(&self) {
        if self.stop.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = self.tx.try_send(Work::Stop);
        self.flush_known();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.role = "none".into();
        state.items.clear();
    }
}
fn point(args: &Value, lang: crate::i18n::Lang) -> Result<(i32, i32), ApiError> {
    let x = helpers::pyint(&args["x"], 0)
        .ok_or_else(|| ApiError::tr("bad_args", lang, "tray.badPosition", crate::i18n::NO_PARAMS))?
        .clamp(-100000, 100000) as i32;
    let y = helpers::pyint(&args["y"], 0)
        .ok_or_else(|| ApiError::tr("bad_args", lang, "tray.badPosition", crate::i18n::NO_PARAMS))?
        .clamp(-100000, 100000) as i32;
    Ok((x, y))
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn interpreter(s: &str) -> bool {
    [
        "node", "nodejs", "java", "bash", "sh", "dash", "env", "bwrap", "gjs", "mono",
    ]
    .contains(&s)
        || ["python", "perl", "ruby", "electron", "wine"]
            .iter()
            .any(|p| {
                s.strip_prefix(p)
                    .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
            })
}
fn exe_name(pid: Option<u32>) -> Option<String> {
    let pid = pid?;
    let exe = fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .file_name()?
        .to_string_lossy()
        .trim_end_matches(" (deleted)")
        .to_string();
    if interpreter(&exe) {
        let argv = fs::read(format!("/proc/{pid}/cmdline"))
            .unwrap_or_default()
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect::<Vec<_>>();
        let mut args = argv.iter().skip(1);
        while let Some(arg) = args.next() {
            if arg == "-m" || arg == "-jar" {
                let name = args
                    .next()
                    .map(|s| {
                        if arg == "-m" {
                            s.clone()
                        } else {
                            Path::new(s)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .trim_end_matches(".jar")
                                .into()
                        }
                    })
                    .unwrap_or_default();
                return Some(helpers::chars(
                    if name.is_empty() { &exe } else { &name },
                    100,
                ));
            }
            if arg == "-c" || arg == "-e" {
                break;
            }
            if arg.starts_with('-') {
                continue;
            }
            let mut name = Path::new(arg)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            for ext in [".py", ".js", ".pl", ".rb", ".jar", ".sh"] {
                name = name.strip_suffix(ext).unwrap_or(&name).into()
            }
            return Some(helpers::chars(
                if name.is_empty() { &exe } else { &name },
                100,
            ));
        }
    }
    (!exe.is_empty()).then(|| helpers::chars(&exe, 100))
}
fn pid_hints(pid: u32) -> Vec<String> {
    let mut hints = Vec::new();
    let env = fs::read(format!("/proc/{pid}/environ"))
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter_map(|b| {
            let i = b.iter().position(|c| *c == b'=')?;
            Some((
                String::from_utf8_lossy(&b[..i]).into_owned(),
                String::from_utf8_lossy(&b[i + 1..]).into_owned(),
            ))
        })
        .collect::<HashMap<_, _>>();
    for key in ["GIO_LAUNCHED_DESKTOP_FILE", "BAMF_DESKTOP_FILE_HINT"] {
        if key == "GIO_LAUNCHED_DESKTOP_FILE"
            && env
                .get("GIO_LAUNCHED_DESKTOP_FILE_PID")
                .is_none_or(|s| s != &pid.to_string())
        {
            continue;
        }
        if let Some(path) = env.get(key) {
            hints.push(
                Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .trim_end_matches(".desktop")
                    .to_string(),
            )
        }
    }
    if let Ok(cgroup) = fs::read_to_string(format!("/proc/{pid}/cgroup")) {
        for path in cgroup.lines().filter_map(|s| s.splitn(3, ':').nth(2)) {
            for unit in path.split('/') {
                if unit.starts_with("app-")
                    && (unit.ends_with(".scope") || unit.ends_with(".service"))
                {
                    let unit = unit
                        .trim_start_matches("app-")
                        .trim_end_matches(".scope")
                        .trim_end_matches(".service");
                    let mut p = unit.split('-').collect::<Vec<_>>();
                    if p.len() > 1 {
                        p.remove(0);
                    }
                    if p.last()
                        .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
                    {
                        p.pop();
                    }
                    hints.push(p.join("-"));
                }
            }
        }
    }
    if let Ok(path) = fs::read_link(format!("/proc/{pid}/exe")) {
        let exe = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim_end_matches(" (deleted)")
            .to_string();
        if !interpreter(&exe) {
            hints.push(exe)
        }
    }
    hints
}

fn fresh_due(item: &mut Item) {
    if item.due.is_some() && item.waiting && item.public.is_none() {
        item.due = None;
        item.waiting = false;
    }
    if item.due.is_some() {
        return;
    }
    let now = Instant::now();
    item.refreshes
        .retain(|t| now.duration_since(*t) < Duration::from_secs(10));
    item.refreshes.push(now);
    item.attempt = 0;
    item.due =
        Some(now + Duration::from_millis(if item.refreshes.len() <= 40 { 50 } else { 1000 }));
}
#[cfg(test)]
mod i18n_tests {
    use super::*;
    use crate::i18n::Lang;
    #[test]
    fn position_errors_render_in_both_languages() {
        // R2-A: the tray stem through point(), the argument check of activate, secondaryActivate and contextMenu
        assert_eq!(point(&json!({"x":3,"y":4}), Lang::En).unwrap(), (3, 4));
        assert_eq!(
            point(&json!({"x":"left"}), Lang::En).unwrap_err().json(),
            json!({"code":"bad_args","message":"Invalid position.","key":"tray.badPosition","params":{}})
        );
        assert_eq!(
            point(&json!({"x":1,"y":[2]}), Lang::Nl)
                .unwrap_err()
                .message,
            "Ongeldige positie."
        );
        // every tray.* row has English that differs from its Dutch
        for key in [
            "tray.invalidIcon",
            "tray.iconGone",
            "tray.notResponding",
            "tray.commandFailed",
            "tray.noMenu",
            "tray.menuFailed",
            "tray.badScrollDirection",
            "tray.badScrollAmount",
            "tray.unknownMenuAction",
            "tray.badMenuItem",
        ] {
            let en = crate::i18n::tr(Lang::En, key);
            assert!(en != key && en != crate::i18n::tr(Lang::Nl, key), "{key}");
        }
        assert_eq!(
            crate::i18n::tr(Lang::En, "tray.noMenu"),
            "This app doesn’t have a menu."
        );
        // the glossary's Windows 11 term "Not responding" (Reageert niet), as Task Manager's hang texts
        assert_eq!(
            crate::i18n::tr(Lang::En, "tray.notResponding"),
            "This app is not responding."
        );
    }
    #[test]
    fn lookup_errors_render_in_the_context_language_through_call() {
        // R2-A fix round (review m4): tray.invalidIcon and tray.iconGone through call() and find(), on the struct
        // itself (no D-Bus thread, like display's en test)
        for (language, invalid, gone) in [
            ("en", "Invalid icon.", "This icon no longer exists."),
            (
                "nl",
                "Ongeldig pictogram.",
                "Dit pictogram bestaat niet meer.",
            ),
        ] {
            let lc = crate::services::system::common::LangContext::new(language, "desktop");
            let ctx = lc.ctx.clone();
            let (tx, _rx) = mpsc::sync_channel(1);
            let tray = Arc::new_cyclic(|weak| Tray {
                cache: icons::Cache::new(ctx.cache_dir.join("tray")),
                ctx,
                weak: weak.clone(),
                state: Mutex::new(State::default()),
                connection: Mutex::new(None),
                tx,
                stop: AtomicBool::new(true),
                host_name: "org.kde.StatusNotifierHost-test".into(),
            });
            let e = tray
                .call("tray.activate", &json!({"x":1,"y":2}))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                e.json(),
                json!({"code":"bad_args","message":invalid,"key":"tray.invalidIcon","params":{}})
            );
            let e = tray
                .call("tray.contextMenu", &json!({"id":"sni:gone","x":1,"y":2}))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                (e.code.as_str(), e.message.as_str(), e.key.as_deref()),
                ("not_found", gone, Some("tray.iconGone"))
            );
        }
    }
}
