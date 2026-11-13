//! UDisks2-backed Schijven. Native equivalents of the original service contracts and guards.
mod actions;
mod checks;
mod efi;
mod labels;
mod model;
#[cfg(test)]
mod tests;
use super::{ServiceContext, system::common::*};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{self, Lang},
};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const UD: &str = "org.freedesktop.UDisks2";
const PATH: &str = "/org/freedesktop/UDisks2";
const OM: &str = "org.freedesktop.DBus.ObjectManager";
const I_BLOCK: &str = "org.freedesktop.UDisks2.Block";
const I_PART: &str = "org.freedesktop.UDisks2.Partition";
const I_TABLE: &str = "org.freedesktop.UDisks2.PartitionTable";
const I_FS: &str = "org.freedesktop.UDisks2.Filesystem";
const I_DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const I_ATA: &str = "org.freedesktop.UDisks2.Drive.Ata";
const I_NVME: &str = "org.freedesktop.UDisks2.NVMe.Controller";
const I_CRYPT: &str = "org.freedesktop.UDisks2.Encrypted";
const I_SWAP: &str = "org.freedesktop.UDisks2.Swapspace";
const I_LOOP: &str = "org.freedesktop.UDisks2.Loop";
const I_JOB: &str = "org.freedesktop.UDisks2.Job";
const I_MANAGER: &str = "org.freedesktop.UDisks2.Manager";
const MIB: i64 = 1024 * 1024;
const MIN_VOLUME: i64 = 8 * MIB;
const METHODS: &[&str] = &[
    "disks.list",
    "disks.mount",
    "disks.unmount",
    "disks.open",
    "disks.setLabel",
    "disks.format",
    "disks.deletePartition",
    "disks.createPartition",
    "disks.initialize",
    "disks.eject",
    "disks.powerOff",
    "disks.unlock",
    "disks.lock",
    "disks.smart",
];
fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn int(v: &Value) -> i64 {
    pyint(v).unwrap_or(0)
}
fn nonzero(v: &Value) -> Option<i64> {
    pyint(v).filter(|i| *i != 0)
}
fn nonempty(v: &Value) -> Option<String> {
    let s = if truth(v) { string(v) } else { String::new() };
    (!s.is_empty()).then_some(s)
}
fn data() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| {
        serde_json::from_str(include_str!("disks/data.json")).expect("embedded disk metadata")
    })
}
fn tr(lang: &str, key: &str, args: &[(&str, String)]) -> String {
    let mut text = data()["_STR"][lang][key]
        .as_str()
        .or_else(|| data()["_STR"]["nl"][key].as_str())
        .unwrap_or(key)
        .to_owned();
    for (k, v) in args {
        text = text.replace(&format!("{{{k}}}"), v);
    }
    text
}
pub(crate) struct Disks {
    ctx: Arc<ServiceContext>,
    readonly: bool,
    stopped: AtomicBool,
    /// disks.list's 1 s snapshot. It holds texts (volume and partition names, the "unavailable" body), so it is
    /// kept with the language it was built in and is used only in that language (round2-gate §2).
    snap: Mutex<Option<(Instant, Lang, Value)>>,
    last: Mutex<Value>,
    pending: Mutex<Option<Instant>>,
    boot: Mutex<Option<(Instant, Vec<Value>)>>,
    stats: StatCache,
    ops: Arc<Mutex<BTreeSet<String>>>,
    #[cfg(test)]
    job_timeout: std::sync::atomic::AtomicI32,
}
impl Disks {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let svc = Arc::new(Self {
            readonly: std::env::var("WIN11SHELL_DISKS_READONLY").is_ok_and(|v| v == "1"),
            ctx,
            stopped: AtomicBool::new(false),
            snap: Mutex::new(None),
            last: Mutex::new(Value::Null),
            pending: Mutex::new(None),
            boot: Mutex::new(None),
            stats: StatCache::new(),
            ops: Arc::new(Mutex::new(BTreeSet::new())),
            #[cfg(test)]
            job_timeout: std::sync::atomic::AtomicI32::new(i32::MAX),
        });
        let weak = Arc::downgrade(&svc);
        thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let Ok(bus) = system() else { return };
                let mut subs = vec![];
                for (sender, iface, signal, path, arg0) in [
                    (UD, OM, None, Some(PATH), None),
                    (
                        UD,
                        "org.freedesktop.DBus.Properties",
                        Some("PropertiesChanged"),
                        None,
                        None,
                    ),
                    (
                        "org.freedesktop.DBus",
                        "org.freedesktop.DBus",
                        Some("NameOwnerChanged"),
                        Some("/org/freedesktop/DBus"),
                        Some(UD),
                    ),
                ] {
                    let weak = weak.clone();
                    subs.push(bus.signal_subscribe(
                        Some(sender),
                        Some(iface),
                        signal,
                        path,
                        arg0,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, _| {
                            if let Some(s) = weak.upgrade() {
                                let mut pending =
                                    s.pending.lock().unwrap_or_else(|e| e.into_inner());
                                if pending.is_none() {
                                    *pending = Some(Instant::now());
                                }
                            }
                        },
                    ));
                }
                if let Some(s) = weak.upgrade() {
                    s.refresh(false);
                }
                loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    let Some(s) = weak.upgrade() else { break };
                    if s.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let pending = s
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_some_and(|at| at.elapsed() >= Duration::from_millis(250));
                    if pending {
                        s.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
                        s.refresh(true);
                    }
                    drop(s);
                    thread::sleep(Duration::from_millis(10));
                }
                for sub in subs {
                    bus.signal_unsubscribe(sub);
                }
            });
        });
        Ok(svc)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
    fn language(&self) -> String {
        self.ctx.language()
    }
    fn err(&self, code: &str, key: &str, args: &[(&str, String)]) -> ApiError {
        ApiError::new(code, &tr(&self.language(), key, args))
    }
    fn dbus(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&glib::Variant>,
        timeout: i32,
        name: &str,
    ) -> Result<glib::Variant, ApiError> {
        if self.readonly && !(iface == OM && method == "GetManagedObjects") {
            return Err(self.err("readonly", "readonly", &[]));
        }
        let bus = system().map_err(|e| self.map_error(&e, name))?;
        bus.call_sync(
            Some(UD),
            path,
            iface,
            method,
            args,
            None,
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            timeout,
            gio::Cancellable::NONE,
        )
        .map_err(|e| self.map_error(&e.to_string(), name))
    }
    fn managed(&self) -> Result<Value, ApiError> {
        self.dbus(PATH, OM, "GetManagedObjects", None, 10000, "")
            .map(|v| first(&v))
    }
    fn options(&self, objects: &Value, lang: Lang) -> model::ParseOptions {
        let mut slaves = BTreeMap::new();
        let mut usage = BTreeMap::new();
        for ifs in objects.as_object().into_iter().flat_map(|o| o.values()) {
            let number = int(&ifs[I_BLOCK]["DeviceNumber"]) as u64;
            if number != 0 {
                let root = PathBuf::from(format!(
                    "/sys/dev/block/{}:{}/slaves",
                    libc::major(number),
                    libc::minor(number)
                ));
                let mut names = std::fs::read_dir(root)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                names.sort();
                slaves.insert(number, names);
            }
            for mount in arr(&ifs[I_FS]["MountPoints"]) {
                let mount = model::bstr(mount);
                if !mount.is_empty() {
                    if let Some(s) = self.stats.get(&mount) {
                        usage.insert(mount, s);
                    }
                }
            }
        }
        let boot = {
            let mut cache = self.boot.lock().unwrap_or_else(|e| e.into_inner());
            if cache
                .as_ref()
                .is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(30))
            {
                *cache = Some((Instant::now(), efi::entries()));
            }
            cache.as_ref().map(|(_, v)| v.clone()).unwrap_or_default()
        };
        let root = self
            .ctx
            .ui
            .parent()
            .unwrap_or(&self.ctx.ui)
            .canonicalize()
            .unwrap_or_else(|_| self.ctx.ui.clone());
        let swap = std::fs::read_to_string("/proc/swaps")
            .unwrap_or_default()
            .lines()
            .skip(1)
            .filter_map(|line| {
                let f = line.split_whitespace().collect::<Vec<_>>();
                (f.get(1) == Some(&"file")).then(|| f[0].replace("\\040", " "))
            })
            .collect();
        model::ParseOptions {
            language: lang.code().into(),
            shell_paths: vec![root.to_string_lossy().into_owned()],
            swap_files: swap,
            boot_entries: boot,
            slaves,
            usage,
            available_tools: labels::tools(),
        }
    }
    fn snapshot(&self, refresh: bool) -> Result<Value, ApiError> {
        let lang = self.ctx.lang();
        if !refresh {
            if let Some((at, built, snap)) =
                self.snap.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            {
                if *built == lang && at.elapsed() < Duration::from_secs(1) {
                    return Ok(snap.clone());
                }
            }
        }
        let snap = match self.managed() {
            Ok(objects) => {
                let mut s = model::parse(&objects, &self.options(&objects, lang));
                s["readOnly"] = json!(self.readonly);
                s
            }
            Err(e)
                if ["unavailable", "timeout", "failed", "not_found"].contains(&e.code.as_str()) =>
            {
                unsupported(lang, e)
            }
            Err(e) => return Err(e),
        };
        *self.snap.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), lang, snap.clone()));
        Ok(snap)
    }
    fn list(&self, refresh: bool) -> Value {
        self.snapshot(refresh).unwrap_or_else(|_|json!({"supported":false,"error":unavailable_body(self.ctx.lang()),"disks":[],"jobs":[],"filesystems":[],"encryption":{"supported":false,"types":[]}}))
    }
    fn refresh(&self, emit: bool) -> Value {
        let snap = self.list(true);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *last != snap;
        *last = snap.clone();
        drop(last);
        if emit && changed {
            self.ctx.emit("disks.changed", snap.clone());
        }
        snap
    }
    fn fresh(&self) -> Result<Value, ApiError> {
        let snap = self.snapshot(true)?;
        if !truth(&snap["supported"]) {
            return Err(ApiError::new(
                "unavailable",
                snap["error"]
                    .as_str()
                    .unwrap_or(&tr(&self.language(), "unavailable", &[])),
            ));
        }
        Ok(snap)
    }
    fn done(&self) {
        self.refresh(true);
    }
    fn timeout(&self) -> i32 {
        #[cfg(test)]
        {
            return self.job_timeout.load(Ordering::Relaxed);
        }
        #[cfg(not(test))]
        {
            i32::MAX
        }
    }
    fn operation(&self, keys: Vec<String>) -> Result<Operation, ApiError> {
        let mut held = self.ops.lock().unwrap_or_else(|e| e.into_inner());
        if keys.iter().any(|k| held.contains(k)) {
            return Err(self.err("op_running", "op_running", &[]));
        }
        held.extend(keys.iter().cloned());
        Ok(Operation {
            locks: self.ops.clone(),
            keys,
            handed_off: false,
        })
    }
    fn job_call(
        self: &Arc<Self>,
        operation: &mut Operation,
        objects: BTreeSet<String>,
        path: &str,
        iface: &str,
        method: &str,
        args: &glib::Variant,
        name: &str,
    ) -> Result<glib::Variant, ApiError> {
        match self.dbus(path, iface, method, Some(args), self.timeout(), name) {
            Err(e) if e.code == "timeout" => {
                let running = self
                    .snapshot(true)
                    .map(|s| !checks::jobs_on(&s, &objects).is_empty())
                    .unwrap_or(false);
                if !running {
                    return Err(e);
                }
                operation.handed_off = true;
                let weak = Arc::downgrade(self);
                let locks = operation.locks.clone();
                let keys = operation.keys.clone();
                thread::spawn(move || {
                    loop {
                        thread::sleep(Duration::from_secs(5));
                        let Some(svc) = weak.upgrade() else { break };
                        if svc.stopped.load(Ordering::Relaxed)
                            || svc
                                .snapshot(true)
                                .map(|s| checks::jobs_on(&s, &objects).is_empty())
                                .unwrap_or(true)
                        {
                            break;
                        }
                    }
                    {
                        let mut locks = locks.lock().unwrap_or_else(|e| e.into_inner());
                        for k in keys {
                            locks.remove(&k);
                        }
                    }
                    if let Some(svc) = weak.upgrade() {
                        svc.done();
                    }
                });
                Err(self.err("still_running", "still_running", &[("name", name.into())]))
            }
            other => other,
        }
    }
    fn map_error(&self, error: &str, name: &str) -> ApiError {
        map_error(&self.language(), error, name)
    }
    pub(crate) fn call(self: &Arc<Self>, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.contains(&method) {
            return None;
        }
        let op = method.strip_prefix("disks.").unwrap();
        if self.ctx.dry_run && !matches!(op, "list" | "open" | "smart") {
            return Some(self.plan(op, args).map(|mut p| {
                p["ok"] = json!(true);
                p["dryRun"] = json!(true);
                p
            }));
        }
        Some(match op {
            "list" => Ok(self.list(truth(&args["refresh"]))),
            "mount" => self.mount(&args["id"]),
            "unmount" => self.unmount(&args["id"]),
            "open" => self.open(&args["id"], self.ctx.dry_run),
            "setLabel" => self.set_label(&args["id"], &args["label"]),
            "format" => self.format(args),
            "deletePartition" => self.delete(args),
            "createPartition" => self.create(args),
            "initialize" => self.initialize(args),
            "eject" => self.remove_drive(args, false),
            "powerOff" => self.remove_drive(args, true),
            "unlock" => self.unlock(&args["id"], &args["passphrase"], args["mount"] != false),
            "lock" => self.lock(&args["id"]),
            "smart" => self.smart(
                if truth(&args["disk"]) {
                    &args["disk"]
                } else {
                    &args["id"]
                },
                truth(&args["refresh"]) && !self.ctx.dry_run,
            ),
            _ => unreachable!(),
        })
    }
}
impl Drop for Disks {
    fn drop(&mut self) {
        self.shutdown();
    }
}
struct Operation {
    locks: Arc<Mutex<BTreeSet<String>>>,
    keys: Vec<String>,
    handed_off: bool,
}
impl Drop for Operation {
    fn drop(&mut self) {
        if !self.handed_off {
            let mut held = self.locks.lock().unwrap_or_else(|e| e.into_inner());
            for k in &self.keys {
                held.remove(k);
            }
        }
    }
}
struct StatCache {
    cache: Mutex<BTreeMap<String, (Instant, Option<(i64, i64, i64)>)>>,
    inflight: Arc<Mutex<BTreeSet<String>>>,
}
impl StatCache {
    fn new() -> Self {
        Self {
            cache: Mutex::new(BTreeMap::new()),
            inflight: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
    fn get(&self, path: &str) -> Option<(i64, i64, i64)> {
        let now = Instant::now();
        if let Some((at, v)) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(path)
        {
            if at.elapsed() < Duration::from_secs(3) {
                return *v;
            }
        }
        let mut busy = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if !busy.insert(path.into()) {
            return None;
        }
        drop(busy);
        let path = path.to_owned();
        let check = path.clone();
        let inflight = self.inflight.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = (|| {
                let p = std::ffi::CString::new(path.as_bytes()).ok()?;
                let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
                if unsafe { libc::statvfs(p.as_ptr(), &mut stat) } != 0 {
                    return None;
                }
                let total = (stat.f_blocks as i128 * stat.f_frsize as i128)
                    .clamp(0, i64::MAX as i128) as i64;
                if total <= 0 {
                    return None;
                }
                let free = (stat.f_bavail as i128 * stat.f_frsize as i128)
                    .clamp(0, i64::MAX as i128) as i64;
                let used = (total as i128 - stat.f_bfree as i128 * stat.f_frsize as i128)
                    .clamp(0, i64::MAX as i128) as i64;
                Some((total, used, free))
            })();
            inflight
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&path);
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(Duration::from_secs(1)).ok().flatten();
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(check, (now, result));
        result
    }
}
/// The Schijven body under its "Schijfbeheer werkt niet" / "Disk Management doesn’t work" heading when UDisks is
/// missing (disks.list `error`). Dutch is 3b898e6's text; the English body does not repeat the heading. The API
/// error `unavailable` keeps _STR's text (map_error), which the reference tests pin in both languages.
fn unavailable_body(lang: Lang) -> String {
    i18n::tr(lang, "disks.unavailableBody")
}
/// disks.list when UDisks cannot be used: `error` is the body Schijven shows under its heading.
fn unsupported(lang: Lang, e: ApiError) -> Value {
    let error = if e.code == "unavailable" {
        unavailable_body(lang)
    } else {
        e.message
    };
    json!({"supported":false,"error":error,"disks":[],"jobs":[],"filesystems":labels::filesystems(lang.code(),None,&labels::tools()),"encryption":{"supported":false,"types":[]}})
}
fn map_error(lang: &str, error: &str, name: &str) -> ApiError {
    let (ename, msg) = if let Some((_, tail)) = error.split_once("GDBus.Error:") {
        tail.split_once(':')
            .map(|(n, m)| (n.to_owned(), m.trim().to_owned()))
            .unwrap_or((String::new(), error.trim().into()))
    } else {
        let name = error
            .find("org.freedesktop.")
            .map(|i| {
                error[i..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_')
                    .collect::<String>()
            })
            .unwrap_or_default();
        (name, error.trim().to_owned())
    };
    let short = ename.rsplit('.').next().unwrap_or("");
    let low = msg.to_lowercase();
    let (code, key) = if ename.ends_with("NotAuthorizedDismissed")
        || short == "Cancelled"
        || low.contains("dismissed")
    {
        ("cancelled", "cancelled")
    } else if short.starts_with("NotAuthorized") || low.contains("not authorized") {
        ("not_authorized", "not_authorized")
    } else if short == "DeviceBusy"
        || low.contains("target is busy")
        || low.contains("device or resource busy")
        || low.contains(" busy")
    {
        ("busy", "busy")
    } else if low.contains("passphrase")
        || low.contains("no key available")
        || (low.contains("operation not permitted") && low.contains("unlock"))
    {
        ("bad_passphrase", "bad_passphrase")
    } else if matches!(short, "ServiceUnknown" | "NameHasNoOwner")
        || low.contains("was not provided by any .service")
    {
        ("unavailable", "unavailable")
    } else if matches!(
        short,
        "UnknownObject" | "UnknownMethod" | "UnknownInterface"
    ) || low.contains("no such interface")
        || low.contains("object does not exist")
    {
        ("not_found", "gone_volume")
    } else if matches!(short, "Timedout" | "Timeout" | "NoReply")
        || low.contains("timeout")
        || low.contains("timed out")
    {
        ("timeout", "timeout")
    } else if short == "NotMounted" {
        ("not_mounted", "not_mounted")
    } else {
        ("failed", "failed")
    };
    let detail = msg
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect::<String>();
    ApiError::new(
        code,
        &tr(
            lang,
            key,
            &[
                (
                    "name",
                    if name.is_empty() {
                        "?".into()
                    } else {
                        name.into()
                    },
                ),
                (
                    "detail",
                    if detail.is_empty() {
                        if short.is_empty() {
                            "Error".into()
                        } else {
                            short.into()
                        }
                    } else {
                        detail
                    },
                ),
            ],
        ),
    )
}
