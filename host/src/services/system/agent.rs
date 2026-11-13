//! Temporary BlueZ agent. Deferred invocations never leave their owning GLib thread.
use super::{common::*, text, text_error};
use crate::{files::ApiResult, services::ServiceContext};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
const MANAGER: &str = "org.bluez.AgentManager1";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LIVE: OnceLock<Mutex<BTreeMap<String, Weak<PairAgent>>>> = OnceLock::new();
fn live() -> &'static Mutex<BTreeMap<String, Weak<PairAgent>>> {
    LIVE.get_or_init(|| Mutex::new(BTreeMap::new()))
}
struct Pending {
    kind: String,
    inv: gio::DBusMethodInvocation,
    token: u64,
    passkey: Option<String>,
    deadline: Instant,
}
thread_local! {static PENDING:RefCell<Option<Pending>>=const{RefCell::new(None)};}
enum Message {
    Reply {
        accept: bool,
        passkey: Value,
        pin: Value,
        qid: Value,
        ack: mpsc::SyncSender<ApiResult>,
    },
    Cancel,
    Stop,
    Finish,
}
pub(super) struct PairAgent {
    ctx: Arc<ServiceContext>,
    address: String,
    device: String,
    pub path: String,
    owner: String,
    tx: mpsc::Sender<Message>,
    pub declined: AtomicBool,
    pub timed_out: AtomicBool,
    released: AtomicBool,
    closing: AtomicBool,
    registered: AtomicBool,
    token: AtomicU64,
}
impl PairAgent {
    pub fn start(
        ctx: Arc<ServiceContext>,
        address: &str,
        device: &str,
    ) -> Result<Arc<Self>, String> {
        let bus = system()?;
        let r = call(
            &bus,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&("org.bluez",).to_variant()),
            2000,
        )?;
        let owner = string(&first(&r));
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let path = format!("/org/win11shell/bluetooth/agent{seq}");
        let (tx, rx) = mpsc::channel();
        let agent = Arc::new(Self {
            ctx,
            address: address.into(),
            device: device.into(),
            path,
            owner,
            tx,
            declined: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            released: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            registered: AtomicBool::new(false),
            token: AtomicU64::new(0),
        });
        let (a, b) = mpsc::sync_channel(1);
        let worker = agent.clone();
        thread::spawn(move || worker.worker(rx, a));
        b.recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())??;
        let params = (object_path(&agent.path)?, "KeyboardDisplay").to_variant();
        let mut result = call(
            &bus,
            "org.bluez",
            "/org/bluez",
            MANAGER,
            "RegisterAgent",
            Some(&params),
            3000,
        );
        if result.as_ref().is_err_and(|e| e.contains("AlreadyExists")) {
            let paths = live()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter_map(|(p, a)| a.upgrade().map(|a| (p.clone(), a)))
                .collect::<Vec<_>>();
            for (path, old) in paths.into_iter().rev().take(32) {
                if unregister(&bus, &path) != Unregister::Failed {
                    old.registered.store(false, Ordering::Relaxed);
                    let _ = old.tx.send(Message::Finish);
                }
            }
            result = call(
                &bus,
                "org.bluez",
                "/org/bluez",
                MANAGER,
                "RegisterAgent",
                Some(&params),
                3000,
            );
        }
        if let Err(e) = result {
            if e.contains("Timeout") || e.contains("NoReply") {
                if unregister(&bus, &agent.path) == Unregister::Failed {
                    agent.registered.store(true, Ordering::Relaxed);
                    live()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(agent.path.clone(), Arc::downgrade(&agent));
                }
            }
            agent.stop();
            return Err(e);
        }
        agent.registered.store(true, Ordering::Relaxed);
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent.path.clone(), Arc::downgrade(&agent));
        let _ = call(
            &bus,
            "org.bluez",
            "/org/bluez",
            MANAGER,
            "RequestDefaultAgent",
            Some(&(object_path(&agent.path)?,).to_variant()),
            3000,
        );
        Ok(agent)
    }
    fn emit(&self, stage: &str, extra: Value) {
        let mut data = json!({"address":self.address,"stage":stage});
        if let (Some(out), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                if !v.is_null() {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        self.ctx.emit("system.bluetooth.pairing", data);
    }
    fn worker(
        self: Arc<Self>,
        rx: mpsc::Receiver<Message>,
        ready: mpsc::SyncSender<Result<(), String>>,
    ) {
        let context = glib::MainContext::new();
        let _ = context.with_thread_default(|| {
            let bus = match system() {
                Ok(b) => b,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            let info = match gio::DBusNodeInfo::for_xml(include_str!("agent.xml")).and_then(|n| {
                n.lookup_interface("org.bluez.Agent1").ok_or_else(|| {
                    glib::Error::new(gio::IOErrorEnum::InvalidData, "agent interface missing")
                })
            }) {
                Ok(v) => v,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            let owner = self.clone();
            let reg = match bus.register_object(
                &self.path,
                &info,
                move |_, sender, _, _, method, params, inv| {
                    owner.handle(sender, method, &variant_json(&params), inv)
                },
                |_, _, _, _, _| ().to_variant(),
                |_, _, _, _, _, _| false,
            ) {
                Ok(id) => id,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            let _ = ready.send(Ok(()));
            let mut finish = false;
            let mut retry: Option<(Instant, usize)> = None;
            while !finish {
                while context.pending() {
                    context.iteration(false);
                }
                if self.released.load(Ordering::Relaxed) && self.closing.load(Ordering::Relaxed) {
                    finish = true;
                }
                while let Ok(message) = rx.try_recv() {
                    match message {
                        Message::Reply {
                            accept,
                            passkey,
                            pin,
                            qid,
                            ack,
                        } => {
                            let _ = ack.send(self.reply_local(accept, &passkey, &pin, &qid));
                        }
                        Message::Cancel => {
                            self.declined.store(true, Ordering::Relaxed);
                            drop_pending();
                        }
                        Message::Finish => finish = true,
                        Message::Stop => {
                            self.closing.store(true, Ordering::Relaxed);
                            drop_pending();
                            if !self.registered.load(Ordering::Relaxed)
                                || self.released.load(Ordering::Relaxed)
                            {
                                finish = true
                            } else if unregister(&bus, &self.path) != Unregister::Failed {
                                self.registered.store(false, Ordering::Relaxed);
                                finish = true
                            } else {
                                retry = Some((Instant::now() + Duration::from_secs(1), 0));
                            }
                        }
                    }
                }
                let expired = PENDING.with(|p| {
                    p.borrow()
                        .as_ref()
                        .is_some_and(|p| Instant::now() >= p.deadline)
                });
                if expired {
                    self.timed_out.store(true, Ordering::Relaxed);
                    drop_pending();
                }
                if let Some((at, n)) = retry {
                    if Instant::now() >= at {
                        if unregister(&bus, &self.path) != Unregister::Failed {
                            self.registered.store(false, Ordering::Relaxed);
                            finish = true;
                            retry = None;
                        } else {
                            retry = match n {
                                0 => Some((Instant::now() + Duration::from_secs(3), 1)),
                                1 => Some((Instant::now() + Duration::from_secs(9), 2)),
                                _ => None,
                            };
                        }
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
            drop_pending();
            let _ = bus.unregister_object(reg);
            live()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.path);
        });
    }
    fn handle(&self, sender: &str, method: &str, args: &Value, inv: gio::DBusMethodInvocation) {
        if self.owner.is_empty() || sender != self.owner {
            reject(inv);
            return;
        }
        if method == "Release" {
            self.released.store(true, Ordering::Relaxed);
            drop_pending();
            inv.return_value(None);
            return;
        }
        if method == "Cancel" {
            if drop_pending() && !self.closing.load(Ordering::Relaxed) {
                self.emit("pairing", json!({}));
            }
            inv.return_value(None);
            return;
        }
        if self.declined.load(Ordering::Relaxed)
            || self.closing.load(Ordering::Relaxed)
            || args[0] != self.device
        {
            reject(inv);
            return;
        }
        match method {
            "RequestConfirmation" => self.hold(
                "confirm",
                inv,
                Some(format!("{:06}", integer(&args[1]).unwrap_or(0) % 1_000_000)),
            ),
            "RequestAuthorization" => self.hold("authorize", inv, None),
            "RequestPasskey" => self.hold("passkey", inv, None),
            "RequestPinCode" => self.hold("pin", inv, None),
            "DisplayPasskey" => {
                inv.return_value(None);
                self.emit("display",json!({"passkey":format!("{:06}",integer(&args[1]).unwrap_or(0)%1_000_000),"entered":integer(&args[2]).unwrap_or(0)}));
            }
            "DisplayPinCode" => {
                inv.return_value(None);
                self.emit("display", json!({"pin":string(&args[1])}));
            }
            "AuthorizeService" => inv.return_value(None),
            _ => reject(inv),
        }
    }
    fn hold(&self, kind: &str, inv: gio::DBusMethodInvocation, passkey: Option<String>) {
        drop_pending();
        let token = self.token.fetch_add(1, Ordering::Relaxed) + 1;
        PENDING.with(|p| {
            *p.borrow_mut() = Some(Pending {
                kind: kind.into(),
                inv,
                token,
                passkey: passkey.clone(),
                deadline: Instant::now() + Duration::from_secs(30),
            })
        });
        self.emit(
            if kind == "authorize" { "confirm" } else { kind },
            json!({"qid":token,"passkey":passkey}),
        );
    }
    pub fn reply(&self, accept: bool, passkey: &Value, pin: &Value, qid: &Value) -> ApiResult {
        let (tx, rx) = mpsc::sync_channel(1);
        if self
            .tx
            .send(Message::Reply {
                accept,
                passkey: passkey.clone(),
                pin: pin.clone(),
                qid: qid.clone(),
                ack: tx,
            })
            .is_err()
        {
            return Ok(json!({"ok":false,"error":text(&self.ctx,"bt_no_request")}));
        }
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| Ok(json!({"ok":false,"error":text(&self.ctx,"bt_no_request")})))
    }
    fn reply_local(&self, accept: bool, passkey: &Value, pin: &Value, qid: &Value) -> ApiResult {
        PENDING.with(|pending| {
            let mut slot = pending.borrow_mut();
            let Some(p) = slot.as_ref() else {
                return Ok(json!({"ok":false,"error":text(&self.ctx,"bt_no_request")}));
            };
            let stale =
                || Ok(json!({"ok":false,"error":text(&self.ctx,"bt_no_request"),"code":"stale"}));
            let given = |v: &Value| !v.is_null() && !string(v).trim().is_empty();
            let mut value = None;
            if accept {
                if !qid.is_null() && string(qid).trim() != p.token.to_string() {
                    return stale();
                }
                match p.kind.as_str() {
                    "passkey" => {
                        if !given(passkey) && given(pin) {
                            return stale();
                        }
                        let s = string(passkey).trim().to_owned();
                        if s.is_empty()
                            || s.chars().count() > 6
                            || !s.chars().all(|c| c.is_ascii_digit())
                        {
                            return Err(text_error(&self.ctx, "bad_args", "bt_bad_passkey"));
                        }
                        value = Some((s.parse::<u32>().unwrap_or(0),).to_variant());
                    }
                    "pin" => {
                        if !given(pin) && given(passkey) {
                            return stale();
                        }
                        let s = string(pin).trim().to_owned();
                        if s.is_empty()
                            || s.len() > 16
                            || !s.bytes().all(|b| b.is_ascii_alphanumeric())
                        {
                            return Err(text_error(&self.ctx, "bad_args", "bt_bad_pin"));
                        }
                        value = Some((s,).to_variant());
                    }
                    _ => {
                        if given(pin) {
                            return stale();
                        }
                        if given(passkey) {
                            let s = string(passkey).trim().to_owned();
                            if p.kind != "confirm"
                                || s.is_empty()
                                || s.chars().count() > 6
                                || !s.bytes().all(|b| b.is_ascii_digit())
                                || p.passkey.as_deref()
                                    != Some(
                                        format!("{:06}", s.parse::<u32>().unwrap_or(0)).as_str(),
                                    )
                            {
                                return stale();
                            }
                        }
                    }
                }
            }
            let p = slot.take().expect("pending checked");
            drop(slot);
            if !accept {
                self.declined.store(true, Ordering::Relaxed);
                reject(p.inv);
            } else {
                p.inv.return_value(value.as_ref());
                self.emit("pairing", json!({}));
            }
            Ok(json!({"ok":true}))
        })
    }
    pub fn cancel(&self) {
        self.declined.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Message::Cancel);
    }
    pub fn stop(&self) {
        self.closing.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Message::Stop);
    }
}
fn reject(inv: gio::DBusMethodInvocation) {
    inv.return_dbus_error("org.bluez.Error.Rejected", "Rejected");
}
fn drop_pending() -> bool {
    PENDING.with(|p| {
        if let Some(p) = p.borrow_mut().take() {
            p.inv
                .return_dbus_error("org.bluez.Error.Canceled", "Canceled");
            true
        } else {
            false
        }
    })
}
#[derive(PartialEq)]
enum Unregister {
    Gone,
    Failed,
}
fn unregister(bus: &gio::DBusConnection, path: &str) -> Unregister {
    let Ok(path) = object_path(path) else {
        return Unregister::Gone;
    };
    match call(
        bus,
        "org.bluez",
        "/org/bluez",
        MANAGER,
        "UnregisterAgent",
        Some(&(path,).to_variant()),
        3000,
    ) {
        Ok(_) => Unregister::Gone,
        Err(e)
            if ["DoesNotExist", "ServiceUnknown", "NameHasNoOwner"]
                .iter()
                .any(|s| e.contains(s)) =>
        {
            Unregister::Gone
        }
        Err(_) => Unregister::Failed,
    }
}
