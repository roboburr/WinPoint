use super::common::*;
use crate::services::ServiceContext;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::CStr,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
pub(super) struct Info {
    cache: Mutex<Option<(Instant, Value)>>,
}
impl Info {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(None),
        }
    }
    pub fn get(&self) -> Value {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut data = if let Some((at, v)) = cache
            .as_ref()
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(120))
        {
            let _ = at;
            v.clone()
        } else {
            let d = collect_info();
            *cache = Some((Instant::now(), d.clone()));
            d
        };
        data["uptime"] = json!(
            read("/proc/uptime")
                .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
                .map(|v| v as u64)
        );
        // the names are read again every time: "Naam van deze pc wijzigen" (machine.setName) changes them
        let (host, pretty) = host_names();
        data["hostname"] = json!(host);
        data["prettyHostname"] = json!(pretty);
        data
    }
}
/// The kernel's host name (uname) and the pretty name as GNOME Settings shows it: hostname1's PrettyHostname
/// ([`PRETTY`], as machine.info reads it), else /etc/machine-info's PRETTY_HOSTNAME, else the host name.
fn host_names() -> (String, String) {
    let uname = unsafe {
        let mut u = std::mem::zeroed::<libc::utsname>();
        libc::uname(&mut u);
        u
    };
    let host = unsafe { CStr::from_ptr(uname.nodename.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    watch_hostname1();
    let hostnamed = PRETTY.get(hostname1_pretty);
    let machine_info = read("/etc/machine-info")
        .map(|s| key_values(&s))
        .and_then(|m| m.get("PRETTY_HOSTNAME").cloned());
    let pretty = pretty_name(hostnamed.as_deref(), machine_info.as_deref(), &host);
    (host, pretty)
}
/// The pretty name: hostname1's, else /etc/machine-info's, else the host name (empty ones do not count).
fn pretty_name(hostnamed: Option<&str>, machine_info: Option<&str>, host: &str) -> String {
    [hostnamed, machine_info]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or(host)
        .to_owned()
}
const HOSTNAME1: &str = "org.freedesktop.hostname1";
const HOSTNAME1_PATH: &str = "/org/freedesktop/hostname1";
/// A value read once and again only after [`Cached::invalidate`] (hostname1's PropertiesChanged).
struct Cached {
    value: Mutex<Option<Option<String>>>,
}
impl Cached {
    const fn new() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }
    fn get(&self, read: impl FnOnce() -> Option<String>) -> Option<String> {
        let mut value = self.value.lock().unwrap_or_else(|e| e.into_inner());
        value.get_or_insert_with(read).clone()
    }
    fn invalidate(&self) {
        *self.value.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}
/// hostname1's PrettyHostname, so `system.info` makes no D-Bus call each time: read at the first call, then again
/// after hostname1's PropertiesChanged ([`watch_hostname1`]). None: hostname1 is not there or has no pretty name.
static PRETTY: Cached = Cached::new();
fn hostname1_pretty() -> Option<String> {
    if cfg!(test) {
        return None; // unit tests never ask the system bus (machine/tests.rs has hostname1's fake)
    }
    let bus = system().ok()?;
    let props = getall(&bus, HOSTNAME1, HOSTNAME1_PATH, HOSTNAME1).ok()?;
    props["PrettyHostname"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
/// A thread of its own (started at the first `system.info`, for the host's life) that follows hostname1's
/// PropertiesChanged: a rename (machine.setName, GNOME Settings, hostnamectl) is read at the next `system.info`.
fn watch_hostname1() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if cfg!(test) {
        return;
    }
    STARTED.call_once(|| {
        let _ = thread::Builder::new()
            .name("win11-hostname1".into())
            .spawn(|| {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let Ok(bus) = system() else { return };
                    let _subscription = bus.signal_subscribe(
                        Some(HOSTNAME1),
                        Some("org.freedesktop.DBus.Properties"),
                        Some("PropertiesChanged"),
                        Some(HOSTNAME1_PATH),
                        Some(HOSTNAME1),
                        gio::DBusSignalFlags::NONE,
                        |_, _, _, _, _, _| PRETTY.invalidate(),
                    );
                    // a change between the first read and the subscription
                    PRETTY.invalidate();
                    loop {
                        context.iteration(true);
                    }
                });
            });
    });
}
fn read(p: impl AsRef<std::path::Path>) -> Option<String> {
    fs::read(p)
        .ok()
        .map(|s| String::from_utf8_lossy(&s).trim().to_owned())
}
fn shell_value(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        t[1..t.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        t.split_whitespace().next().unwrap_or("").into()
    }
}
fn key_values(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().into(), shell_value(v)))
        })
        .collect()
}
pub(super) fn meminfo() -> BTreeMap<String, u64> {
    read("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            let fields = v.split_whitespace().collect::<Vec<_>>();
            let n = fields.first()?.parse::<u64>().ok()?;
            Some((k.into(), n * if fields.len() > 1 { 1024 } else { 1 }))
        })
        .collect()
}
fn physical_memory() -> Option<u64> {
    let root = PathBuf::from("/sys/devices/system/memory");
    let block = u64::from_str_radix(&read(root.join("block_size_bytes"))?, 16).ok()?;
    let count = fs::read_dir(&root)
        .ok()?
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("memory")
                && read(e.path().join("online")).unwrap_or_else(|| "1".into()) == "1"
        })
        .count();
    (count > 0).then_some(block * count as u64)
}
fn bad_dmi(s: &str) -> bool {
    let s = s.to_lowercase();
    s.is_empty()
        || s.starts_with("to be filled")
        || [
            "system product name",
            "system manufacturer",
            "default string",
            "o.e.m",
            "o.e.m.",
            "not applicable",
            "none",
            "n/a",
        ]
        .contains(&s.as_str())
}
fn device_model(hostname: &str) -> String {
    let root = PathBuf::from("/sys/class/dmi/id");
    let mut vendor = read(root.join("sys_vendor")).unwrap_or_default();
    let mut product = read(root.join("product_name")).unwrap_or_default();
    let version = read(root.join("product_version")).unwrap_or_default();
    if bad_dmi(&product) {
        vendor = read(root.join("board_vendor")).unwrap_or(vendor);
        product = read(root.join("board_name")).unwrap_or_default()
    }
    if bad_dmi(&vendor) {
        vendor.clear()
    }
    if !product.is_empty()
        && !version.is_empty()
        && !bad_dmi(&version)
        && !product.to_lowercase().contains(&version.to_lowercase())
        && vendor.to_lowercase().contains("lenovo")
    {
        product = version
    }
    for suffix in [
        " Co.",
        " Co,",
        " Inc",
        " Corporation",
        " Corp",
        " GmbH",
        " Technology",
    ] {
        if let Some(i) = vendor.find(suffix) {
            vendor.truncate(i)
        }
    }
    let result = format!("{} {}", vendor.trim(), product.trim())
        .trim()
        .to_owned();
    if result.is_empty() {
        hostname.into()
    } else {
        result
    }
}
fn cpu_info() -> Value {
    let mut model = String::new();
    let mut threads = 0;
    let mut physical = None;
    let mut core = None;
    let mut cores = HashSet::new();
    for line in read("/proc/cpuinfo").unwrap_or_default().lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            match k.trim() {
                "processor" => threads += 1,
                "model name" if model.is_empty() => model = v.into(),
                "physical id" => physical = Some(v.to_owned()),
                "core id" => core = Some(v.to_owned()),
                _ => {}
            }
        } else {
            if physical.is_some() || core.is_some() {
                cores.insert((physical.take(), core.take()));
            }
        }
    }
    if physical.is_some() || core.is_some() {
        cores.insert((physical, core));
    }
    if threads == 0 {
        threads = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(1)
    }
    model = model
        .replace("(R)", "")
        .replace("(TM)", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    json!({"model":if model.is_empty(){"Processor".into()}else{model},"cores":if cores.is_empty(){threads}else{cores.len()},"threads":threads})
}
fn disks() -> Vec<Value> {
    let r = run(
        &[
            "lsblk",
            "-J",
            "-b",
            "-d",
            "-o",
            "NAME,MODEL,SIZE,TYPE,TRAN,ROTA,RM",
        ],
        4,
        false,
    );
    if !r.ok {
        return vec![];
    }
    let data: Value = serde_json::from_str(&r.out).unwrap_or(Value::Null);
    data["blockdevices"].as_array().unwrap_or(&vec![]).iter().filter(|d|d["type"]=="disk"&&!["zram","loop","ram","md","sr"].iter().any(|p|string(&d["name"]).starts_with(p))).map(|d|{let name=string(&d["name"]);let tran=string(&d["tran"]).to_lowercase();let kind=if tran=="nvme"||name.starts_with("nvme"){"nvme"}else if tran=="usb"{"usb"}else if d["rota"]==true||d["rota"]=="1"||d["rota"]==1{"hdd"}else{"ssd"};json!({"model":d["model"].as_str().filter(|s|!s.is_empty()).unwrap_or(&name).trim(),"size":integer(&d["size"]).unwrap_or(0),"kind":kind,"device":name})}).collect()
}
fn collect_info() -> Value {
    let os = read("/etc/os-release")
        .or_else(|| read("/usr/lib/os-release"))
        .map(|s| key_values(&s))
        .unwrap_or_default();
    let uname = unsafe {
        let mut u = std::mem::zeroed::<libc::utsname>();
        libc::uname(&mut u);
        u
    };
    let cstr = |p: *const libc::c_char| unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    let (host, pretty) = host_names();
    let (username, real) = unsafe {
        let pw = libc::getpwuid(libc::geteuid());
        if pw.is_null() {
            (std::env::var("USER").unwrap_or_default(), String::new())
        } else {
            (
                cstr((*pw).pw_name),
                cstr((*pw).pw_gecos)
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            )
        }
    };
    let home = glib::home_dir();
    let avatar = [
        PathBuf::from("/var/lib/AccountsService/icons").join(&username),
        home.join(".face"),
        home.join(".face.icon"),
    ]
    .into_iter()
    .find(|p| {
        p.is_file() && fs::metadata(p).is_ok_and(|m| m.len() > 0) && fs::File::open(p).is_ok()
    });
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .map(|s| s.split(':').next().unwrap_or("").to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Win11 Shell".into());
    let installed = ["/var/log/installer", "/etc/machine-id"]
        .into_iter()
        .find_map(|p| {
            fs::metadata(p)
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as u64)
        });
    let gpus = crate::services::sysmon::gpu_names();
    json!({"hostname":host,"prettyHostname":pretty,"os":os.get("PRETTY_NAME").cloned().unwrap_or_else(||cstr(uname.sysname.as_ptr())),"osVersion":os.get("VERSION").or_else(||os.get("VERSION_ID")).cloned().unwrap_or_default(),"osId":os.get("ID").cloned().unwrap_or_default(),"kernel":cstr(uname.release.as_ptr()),"architecture":cstr(uname.machine.as_ptr()),"deviceModel":device_model(&host),"cpu":cpu_info(),"memory":{"total":meminfo().get("MemTotal").copied().unwrap_or(0),"installed":physical_memory()},"gpus":gpus,"disks":disks(),"uptime":null,"user":{"name":username,"realName":if real.is_empty(){username.clone()}else{real},"avatar":avatar},"desktop":if desktop.eq_ignore_ascii_case("gnome"){"GNOME".into()}else{desktop},"sessionType":std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_|if std::env::var_os("WAYLAND_DISPLAY").is_some(){"wayland".into()}else{"x11".into()}),"installedAt":installed})
}
pub(super) struct Updates {
    ctx: Arc<ServiceContext>,
    state: Mutex<(Option<Value>, Option<Instant>, bool)>,
}
impl Updates {
    pub fn new(ctx: Arc<ServiceContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            state: Mutex::new((None, None, false)),
        })
    }
    pub fn get(self: &Arc<Self>, refresh: bool) -> Value {
        // design §4.10: once a snapshot exists it is the only count (no apt list, no event from here)
        if let Some(count) = crate::services::update::system_updates(&self.ctx) {
            return count;
        }
        if !available("apt") {
            return json!({"available":null,"checkedAt":null,"tool":null,"checking":false,
                          "rebootRequired":crate::services::update::reboot_required()});
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.2
            && (s.0.is_none()
                || refresh
                || s.1.is_none_or(|t| t.elapsed() > Duration::from_secs(1800)))
        {
            s.2 = true;
            let weak = Arc::downgrade(self);
            thread::spawn(move || {
                if let Some(s) = weak.upgrade() {
                    s.check();
                }
            });
        }
        let mut r =
            s.0.clone()
                .unwrap_or(json!({"available":null,"checkedAt":null,"tool":"apt","security":null}));
        r["checking"] = json!(s.2);
        r["rebootRequired"] = json!(crate::services::update::reboot_required());
        r
    }
    fn check(&self) {
        // a snapshot appeared while this apt run was queued: the owner counts and emits now (design §4.4)
        if crate::services::update::system_updates(&self.ctx).is_some() {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).2 = false;
            return;
        }
        let r = run(&["apt", "list", "--upgradable"], 60, true);
        let lines = r
            .out
            .lines()
            .filter(|l| l.contains("[upgradable from"))
            .collect::<Vec<_>>();
        let result = json!({"available":if r.ok{Some(lines.len())}else{None},"checkedAt":now_ms(),"tool":"apt","security":if r.ok{Some(lines.iter().filter(|l|l.split(' ').next().unwrap_or("").contains("-security")).count())}else{None},"checking":false});
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let changed = state
            .0
            .as_ref()
            .is_none_or(|old| old["available"] != result["available"]);
        *state = (Some(result.clone()), Some(Instant::now()), false);
        drop(state);
        if changed {
            self.ctx.emit("system.updates.changed", result);
        }
    }
}
pub(super) fn time_info(h24: Option<bool>) -> Value {
    let mut props = BTreeMap::new();
    let r = run(&["timedatectl", "show"], 4, true);
    if r.ok {
        props = key_values(&r.out)
    }
    let timezone = props
        .get("Timezone")
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            fs::canonicalize("/etc/localtime").ok().and_then(|p| {
                p.to_string_lossy()
                    .split_once("/zoneinfo/")
                    .map(|(_, v)| v.to_owned())
            })
        })
        .unwrap_or_else(|| "UTC".into());
    static H24: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    let system = *H24.get_or_init(|| {
        let r = run(&["locale", "t_fmt"], 2, false);
        r.ok.then(|| !["%I", "%r", "%l", "%p"].iter().any(|p| r.out.contains(p)))
    });
    let yes = |key: &str| props.get(key).is_some_and(|v| v == "yes");
    json!({"timezone":timezone,"ntp":yes("NTP"),"h24":h24.or(system).unwrap_or(true),"ntpSynced":yes("NTPSynchronized"),"canNTP":yes("CanNTP"),"localRTC":yes("LocalRTC"),"systemH24":system,"now":now_ms()})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_strings_unquote() {
        assert_eq!(
            key_values("# hi\nNAME=\"Ubuntu Test\"\nVERSION='one two'\n")["NAME"],
            "Ubuntu Test"
        );
        assert_eq!(shell_value("''"), "");
    }
    #[test]
    fn the_pretty_name_is_hostname1s_then_machine_infos_then_the_host_name() {
        assert_eq!(pretty_name(Some("Robins pc"), Some("Oud"), "robin-pc"), "Robins pc");
        assert_eq!(pretty_name(None, Some("Oud"), "robin-pc"), "Oud");
        assert_eq!(pretty_name(Some(" "), Some(""), "robin-pc"), "robin-pc");
        assert_eq!(pretty_name(None, None, "robin-pc"), "robin-pc");
        // read once, again only after hostname1's PropertiesChanged
        let cache = Cached::new();
        let reads = std::cell::Cell::new(0);
        let read = |name: &str| {
            reads.set(reads.get() + 1);
            Some(name.to_owned())
        };
        assert_eq!(cache.get(|| read("Eerste")), Some("Eerste".into()));
        assert_eq!(cache.get(|| read("Tweede")), Some("Eerste".into()));
        assert_eq!(reads.get(), 1, "no D-Bus call for each system.info");
        cache.invalidate();
        assert_eq!(cache.get(|| read("Tweede")), Some("Tweede".into()));
        assert_eq!(reads.get(), 2);
        // no hostname1: remembered too, until a change
        let none = Cached::new();
        assert_eq!(none.get(|| None), None);
        assert_eq!(none.get(|| Some("x".into())), None);
    }
    #[test]
    fn dmi_rejects_placeholders() {
        assert!(bad_dmi("To Be Filled By O.E.M."));
        assert!(!bad_dmi("ThinkPad T14"));
    }
    /// GW2c (design §4.10): once the update service wrote a finished check, `system.updates` is its count of
    /// offered packages (phased and held left out), with its `checkedAt`, `tool:'packagekit'` and
    /// `rebootRequired`; neither `get` nor `check` starts `apt list` then (GE0b's guard log stays empty).
    #[test]
    fn system_updates_come_from_the_update_snapshot_once_one_exists() {
        use crate::services::update::store;
        let c = LangContext::new("en", "settings");
        let s = store::Store::open(c.ctx.cache_dir.join("update")).unwrap();
        let pkg = |name: &str, info: u32, data: &str, phased: bool, held: bool| {
            json!({"id":format!("{name};1.0-1;amd64;{data}"),"info":info,"summary":"","phased":phased,"held":held})
        };
        s.write(
            store::SNAPSHOT,
            &json!({"version":1,"rev":3,"checkedAt":1762341720000u64,"backend":"packagekit","packages":[
                pkg("libsoup-3.0-0", 8, "ubuntu-resolute-security", false, false),
                pkg("tzdata", 5, "ubuntu-resolute-updates", false, false),
                pkg("vim-tiny", 5, "ubuntu-resolute-updates", false, false),
                pkg("fwupd", 5, "ubuntu-resolute-updates", true, false),
                pkg("zsh", 9, "ubuntu-resolute-updates", false, true)]}),
        )
        .unwrap();
        let updates = Updates::new(c.ctx.clone());
        let r = updates.get(true);
        assert_eq!(r["available"], 3);
        assert_eq!(r["security"], 1);
        assert_eq!(r["checkedAt"], 1762341720000u64);
        assert_eq!(r["tool"], "packagekit");
        assert_eq!(r["checking"], false);
        assert_eq!(r["rebootRequired"], crate::services::update::reboot_required());
        // no apt run was started or recorded, and check() leaves the snapshot's count alone
        updates.check();
        let state = updates.state.lock().unwrap();
        assert!(state.0.is_none() && state.1.is_none() && !state.2);
    }
}
