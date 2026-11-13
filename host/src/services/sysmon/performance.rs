use super::{ServiceContext, common::*};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
pub(super) fn cpu_times() -> Vec<(u64, u64)> {
    let data = read("/proc/stat");
    let mut out = vec![];
    for l in data.lines() {
        if !l.starts_with("cpu") {
            break;
        }
        let v = l
            .split_whitespace()
            .skip(1)
            .take(8)
            .filter_map(|v| v.parse::<u64>().ok())
            .collect::<Vec<_>>();
        if v.len() < 5 {
            continue;
        }
        let total = v.iter().sum::<u64>();
        out.push((total.saturating_sub(v[3] + v[4]), total));
    }
    out
}
pub(super) fn meminfo() -> BTreeMap<String, u64> {
    read("/proc/meminfo")
        .lines()
        .filter_map(|l| {
            let (k, r) = l.split_once(':')?;
            let f = r.split_whitespace().collect::<Vec<_>>();
            Some((
                k.into(),
                f.first()?
                    .parse::<u64>()
                    .ok()?
                    .saturating_mul(if f.len() > 1 { 1024 } else { 1 }),
            ))
        })
        .collect()
}
pub(super) fn diskstats() -> BTreeMap<String, [u64; 5]> {
    read("/proc/diskstats")
        .lines()
        .filter_map(|l| {
            let p = l.split_whitespace().collect::<Vec<_>>();
            if p.len() < 14 {
                return None;
            }
            let n = |i: usize| p[i].parse::<u64>().ok();
            Some((
                p[2].into(),
                [
                    n(5)?,
                    n(9)?,
                    n(12)?,
                    n(3)?.saturating_add(n(7)?),
                    n(6)?.saturating_add(n(10)?),
                ],
            ))
        })
        .collect()
}
pub(super) fn netdev() -> BTreeMap<String, [u64; 2]> {
    read("/proc/net/dev")
        .lines()
        .skip(2)
        .filter_map(|l| {
            let (n, r) = l.split_once(':')?;
            let f = r.split_whitespace().collect::<Vec<_>>();
            if f.len() < 9 {
                return None;
            }
            Some((n.trim().into(), [f[0].parse().ok()?, f[8].parse().ok()?]))
        })
        .collect()
}
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while !a.is_empty() && !b.is_empty() {
        if a[0].is_ascii_digit() && b[0].is_ascii_digit() {
            let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let av = std::str::from_utf8(&a[..na])
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0);
            let bv = std::str::from_utf8(&b[..nb])
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0);
            if av != bv {
                return av.cmp(&bv);
            }
            a = &a[na..];
            b = &b[nb..];
        } else {
            if a[0] != b[0] {
                return a[0].cmp(&b[0]);
            }
            a = &a[1..];
            b = &b[1..];
        }
    }
    a.len().cmp(&b.len())
}
pub(super) fn physical_disks() -> Vec<String> {
    let mut out = names("/sys/block")
        .into_iter()
        .filter(|n| {
            !["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"]
                .iter()
                .any(|s| n.starts_with(s))
                && !(n.starts_with("mmcblk") && (n.contains("boot") || n.contains("rpmb")))
                && Path::new(&format!("/sys/block/{n}/device")).exists()
                && int(read(format!("/sys/block/{n}/size"))).unwrap_or(0) > 0
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        (!a.starts_with("nvme"))
            .cmp(&!b.starts_with("nvme"))
            .then_with(|| natural(a, b))
    });
    out
}
fn mount_map() -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::<String, Vec<String>>::new();
    for l in read("/proc/self/mountinfo").lines() {
        let p = l.split(' ').collect::<Vec<_>>();
        if p.len() < 10 {
            continue;
        }
        let Some(sep) = p.iter().position(|s| *s == "-") else {
            continue;
        };
        let Some(kind) = p.get(sep + 1) else { continue };
        if ["squashfs", "overlay", "tmpfs", "proc", "sysfs", "devtmpfs"].contains(kind)
            || p[4].starts_with("/snap/")
        {
            continue;
        }
        let mp = p[4].replace("\\040", " ");
        if ["/var/lib/docker", "/run/user", "/proc", "/sys"]
            .iter()
            .any(|s| mp.starts_with(s))
        {
            continue;
        }
        out.entry(p[2].into()).or_default().push(mp);
    }
    out
}
fn disk_mounts(disk: &str, mm: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut devs = vec![disk.to_owned()];
    devs.extend(
        names(format!("/sys/block/{disk}"))
            .into_iter()
            .filter(|s| s.starts_with(disk)),
    );
    for dm in names("/sys/block") {
        if dm.starts_with("dm-")
            && names(format!("/sys/block/{dm}/slaves"))
                .iter()
                .any(|s| s == disk || s.starts_with(disk))
        {
            devs.push(dm)
        }
    }
    let mut mounts = vec![];
    for d in devs {
        let base = if d != disk && !d.starts_with("dm-") {
            format!("/sys/block/{disk}/{d}")
        } else {
            format!("/sys/block/{d}")
        };
        let dev = read(format!("{base}/dev"));
        if let Some(a) = mm.get(&dev) {
            for m in a {
                if !mounts.contains(m) {
                    mounts.push(m.clone());
                }
            }
        }
    }
    mounts.sort_by(|a, b| {
        (a != "/")
            .cmp(&(b != "/"))
            .then(a.len().cmp(&b.len()))
            .then(a.cmp(b))
    });
    mounts
}
fn udev_model(name: &str) -> Option<String> {
    let dev = read(format!("/sys/block/{name}/dev"));
    if dev.is_empty() {
        return None;
    }
    let data = read(format!("/run/udev/data/b{dev}"));
    if let Some(enc) = data.lines().find_map(|l| l.strip_prefix("E:ID_MODEL_ENC=")) {
        let v = unescape(enc)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !v.is_empty() {
            return Some(v);
        }
    }
    data.lines().find_map(|l| {
        l.strip_prefix("E:ID_MODEL=")
            .map(|s| s.replace('_', " ").trim().to_owned())
    })
}
fn disk_info(name: &str) -> Value {
    let base = PathBuf::from(format!("/sys/block/{name}"));
    let rot = read(base.join("queue/rotational")) == "1";
    let real = fs::canonicalize(&base)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let udev = udev_model(name);
    let mut model = udev.clone().unwrap_or_else(|| {
        let a = read(base.join("device/model"));
        if !a.is_empty() {
            a
        } else {
            read(base.join("device/name"))
        }
    });
    if model.trim().is_empty() {
        model = name.into()
    }
    let vendor = if udev.is_some() {
        String::new()
    } else {
        read(base.join("device/vendor"))
    };
    if !vendor.is_empty()
        && vendor.to_uppercase() != "ATA"
        && vendor.to_uppercase() != "NVME"
        && !model.to_lowercase().starts_with(&vendor.to_lowercase())
    {
        model = format!("{vendor} {model}")
    }
    json!({"model":model,"kind":if name.starts_with("nvme"){"nvme"}else if real.contains("/usb"){"usb"}else if rot{"hdd"}else{"ssd"},"removable":read(base.join("removable"))=="1"})
}
fn ip_addrs() -> BTreeMap<String, Value> {
    let r = run("ip", &["-j", "addr"], 3.);
    let data = if r.ok {
        serde_json::from_str::<Vec<Value>>(&r.out).unwrap_or_default()
    } else {
        vec![]
    };
    let mut out = BTreeMap::new();
    for itf in data {
        let name = itf["ifname"].as_str().unwrap_or("");
        let (mut v4, mut v6, mut v6ll) = (Value::Null, Value::Null, Value::Null);
        for a in itf["addr_info"].as_array().into_iter().flatten() {
            if a["family"] == "inet" && v4.is_null() {
                v4 = a["local"].clone()
            } else if a["family"] == "inet6" {
                if a["scope"] == "global" && v6.is_null() {
                    v6 = a["local"].clone()
                } else if a["scope"] == "link" && v6ll.is_null() {
                    v6ll = a["local"].clone()
                }
            }
        }
        out.insert(name.into(),json!({"ipv4":v4,"ipv6":if v6.is_null(){v6ll}else{v6},"mac":itf["address"],"state":itf["operstate"]}));
    }
    out
}
fn adapters() -> Vec<(String, String, String)> {
    let mut names = names("/sys/class/net");
    names.sort();
    let mut out = vec![];
    for name in names {
        let base = PathBuf::from(format!("/sys/class/net/{name}"));
        let kind = if base.join("wireless").is_dir() || base.join("phy80211").is_dir() {
            "wifi"
        } else if base.join("device").exists() {
            "ethernet"
        } else if ["tailscale", "wg", "tun", "zt", "nordlynx", "proton", "ppp"]
            .iter()
            .any(|s| name.starts_with(s))
        {
            if read(base.join("operstate")) == "down" {
                continue;
            }
            "other"
        } else {
            continue;
        };
        let slot = if kind != "other" {
            fs::canonicalize(base.join("device"))
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "device".into())
        } else {
            String::new()
        };
        out.push((name, kind.into(), slot));
    }
    out
}
pub(super) fn nice_pci(vendor: &str, device: &str) -> String {
    let v = if vendor.contains("NVIDIA") {
        "NVIDIA".to_owned()
    } else if ["Advanced Micro Devices", "AMD", "ATI"]
        .iter()
        .any(|s| vendor.contains(s))
    {
        "AMD".into()
    } else if vendor.contains("Intel") {
        "Intel".into()
    } else {
        let words = vendor.split_whitespace().collect::<Vec<_>>();
        let end = words
            .iter()
            .position(|s| {
                [
                    "Corporation",
                    "Corp",
                    "Corp.",
                    "Inc",
                    "Inc.",
                    "Co.,",
                    "Co.",
                    "Technology",
                    "Semiconductor",
                ]
                .contains(s)
            })
            .unwrap_or(words.len());
        words[..end].join(" ")
    };
    let model = if let Some((_, s)) = device.split_once('[') {
        s.split_once(']').map(|p| p.0).unwrap_or(device)
    } else {
        device
    };
    if !v.is_empty() && model.to_lowercase().starts_with(&v.to_lowercase()) {
        model.into()
    } else {
        format!("{v} {model}").trim().into()
    }
}
fn pci_name(slot: &str) -> Option<String> {
    if !command_exists("lspci") {
        return None;
    }
    let r = run("lspci", &["-mm", "-s", slot], 3.);
    if !r.ok {
        return None;
    }
    let f = glib::shell_parse_argv(r.out.lines().next()?).ok()?;
    Some(nice_pci(
        f.get(2).and_then(|s| s.to_str()).unwrap_or(""),
        f.get(3).and_then(|s| s.to_str()).unwrap_or(""),
    ))
}
fn drm_cards() -> Vec<(String, PathBuf, String, String)> {
    let mut ns = names("/sys/class/drm");
    ns.sort();
    let mut out = vec![];
    for n in ns {
        if !n
            .strip_prefix("card")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let dev = PathBuf::from(format!("/sys/class/drm/{n}/device"));
        let vendor = match read(dev.join("vendor")).to_lowercase().as_str() {
            "0x10de" => "nvidia",
            "0x1002" => "amd",
            "0x8086" => "intel",
            _ => "other",
        };
        let slot = fs::canonicalize(&dev)
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "device".into());
        out.push((n, dev, vendor.into(), slot));
    }
    out
}
#[derive(Default)]
struct GpuState {
    nv: Option<Vec<Value>>,
    time: f64,
    busy: bool,
}
pub(super) struct Gpu {
    state: Mutex<GpuState>,
    event: Condvar,
}
impl Gpu {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GpuState::default()),
            event: Condvar::new(),
        })
    }
    pub fn nvidia(self: &Arc<Self>, max_age: f64, wait: f64) -> Vec<Value> {
        if !command_exists("nvidia-smi") {
            return vec![];
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = state.nv.is_some() && now() - state.time < max_age;
        let first = state.nv.is_none();
        if !fresh && !state.busy {
            state.busy = true;
            let gpu = self.clone();
            std::thread::spawn(move || {
                let r = run(
                    "nvidia-smi",
                    &[
                        "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu,utilization.encoder,utilization.decoder,power.draw,clocks.gr,driver_version,pci.bus_id",
                        "--format=csv,noheader,nounits",
                    ],
                    3.,
                );
                let out = if r.ok { parse_nvidia(&r.out) } else { vec![] };
                let mut state = gpu.state.lock().unwrap_or_else(|e| e.into_inner());
                state.nv = Some(out);
                state.time = now();
                state.busy = false;
                gpu.event.notify_all();
            });
        }
        if first {
            let result = self
                .event
                .wait_timeout_while(state, Duration::from_secs_f64(wait), |s| s.busy);
            state = result.unwrap_or_else(|e| e.into_inner()).0;
        }
        state.nv.clone().unwrap_or_default()
    }
    pub fn stats(self: &Arc<Self>) -> Vec<Value> {
        let mut out = self.nvidia(1., 1.5);
        let done = !out.is_empty();
        for (card, dev, vendor, slot) in drm_cards() {
            if vendor == "nvidia" && (done || command_exists("nvidia-smi")) {
                continue;
            }
            let usage = int(read(dev.join("gpu_busy_percent")));
            let used = int(read(dev.join("mem_info_vram_used")));
            let total = int(read(dev.join("mem_info_vram_total")));
            let mut hw = names(dev.join("hwmon"));
            hw.sort();
            let temp = hw.into_iter().find_map(|h| {
                int(read(dev.join("hwmon").join(h).join("temp1_input")))
                    .map(|t| round(t as f64 / 1000., 1))
            });
            out.push(json!({"name":pci_name(&slot).unwrap_or(card),"usage":usage,"memUsed":used,"memTotal":total,"temperature":temp,"vendor":vendor,"slot":slot}));
        }
        out
    }
}
pub(super) fn parse_nvidia(data: &str) -> Vec<Value> {
    data.trim().lines().filter_map(|line|{let f=line.split(',').map(str::trim).collect::<Vec<_>>();if f.len()<5{return None}let n=|i:usize|f.get(i).and_then(|s|num(s));let name=if f[0].to_uppercase().starts_with("NVIDIA"){f[0].to_owned()}else{format!("NVIDIA {}",f[0])};Some(json!({"name":name,"usage":n(1),"memUsed":n(2).map(|v|(v*1048576.)as i64),"memTotal":n(3).map(|v|(v*1048576.)as i64),"temperature":n(4),"vendor":"nvidia","engines":{"3d":n(1),"encode":n(5),"decode":n(6)},"power":n(7),"clockMHz":n(8),"driver":f.get(9),"slot":f.get(10).map(|s|{let s=s.to_lowercase();s.chars().skip(s.chars().count().saturating_sub(12)).collect::<String>()})}))}).collect()
}
pub(super) fn cpu_temperature() -> Option<f64> {
    let mut hs = names("/sys/class/hwmon");
    hs.sort();
    let mut best: Option<(i32, i32, PathBuf)> = None;
    for h in hs {
        if !h.starts_with("hwmon") {
            continue;
        }
        let base = PathBuf::from("/sys/class/hwmon").join(h);
        let name = read(base.join("name"));
        let rank = match name.as_str() {
            "k10temp" | "zenpower" | "coretemp" => 0,
            "cpu_thermal" => 1,
            "acpitz_0" | "acpitz" => 2,
            _ => continue,
        };
        let mut labels = names(&base);
        labels.sort();
        for l in labels {
            if !l.starts_with("temp") || !l.ends_with("_label") {
                continue;
            }
            let label = read(base.join(&l));
            let lr = match label.as_str() {
                "Tdie" | "Package id 0" => 0,
                "Tctl" => 1,
                _ => 5,
            };
            let cand = (rank, lr, base.join(l.replace("_label", "_input")));
            if best.as_ref().is_none_or(|b| &cand < b) {
                best = Some(cand)
            }
        }
        if best.as_ref().is_none_or(|b| b.0 > rank) {
            let input = base.join("temp1_input");
            if input.exists() {
                let cand = (rank, 6, input);
                if best.as_ref().is_none_or(|b| &cand < b) {
                    best = Some(cand)
                }
            }
        }
    }
    best.and_then(|(_, _, p)| int(read(p)))
        .map(|t| round(t as f64 / 1000., 1))
}
fn cpu_static() -> Value {
    let data = read("/proc/cpuinfo");
    let (mut model, mut phys, mut core) = (String::new(), None::<String>, None::<String>);
    let mut cores = BTreeSet::new();
    let mut threads = 0;
    for line in data.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            match k {
                "processor" => threads += 1,
                "model name" if model.is_empty() => model = v.into(),
                "physical id" => phys = Some(v.into()),
                "core id" => core = Some(v.into()),
                _ => {}
            }
        } else {
            if phys.is_some() || core.is_some() {
                cores.insert((phys.take(), core.take()));
            }
            phys = None;
            core = None;
        }
    }
    if phys.is_some() || core.is_some() {
        cores.insert((phys, core));
    }
    model = model
        .replace("(R)", "")
        .replace("(TM)", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if threads == 0 {
        threads = ncpu()
    }
    json!({"model":if model.is_empty(){"Processor".into()}else{model},"cores":if cores.is_empty(){threads}else{cores.len()},"threads":threads,"maxMHz":int(read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")).filter(|v|*v!=0).map(|v|round(v as f64/1000.,0)as i64),"baseMHz":int(read("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")).filter(|v|*v!=0).map(|v|round(v as f64/1000.,0)as i64)})
}
pub(super) struct Performance {
    cpu_prev: Vec<(u64, u64)>,
    disk_prev: (f64, BTreeMap<String, [u64; 5]>),
    net_prev: (f64, BTreeMap<String, [u64; 2]>),
    cpuinfo: Option<Value>,
    primary: (f64, Value),
    ips: (f64, BTreeMap<String, Value>),
}
impl Performance {
    pub fn new() -> Self {
        Self {
            cpu_prev: cpu_times(),
            disk_prev: (now(), diskstats()),
            net_prev: (now(), netdev()),
            cpuinfo: None,
            primary: (0., Value::Null),
            ips: (0., BTreeMap::new()),
        }
    }
    pub fn get(&mut self, ctx: &Arc<ServiceContext>, gpu: &Arc<Gpu>) -> Value {
        let time = now();
        let lang = ctx.language();
        let times = cpu_times();
        let usage = times
            .iter()
            .enumerate()
            .map(|(i, (busy, total))| {
                let (pb, pt) = self.cpu_prev.get(i).copied().unwrap_or((*busy, *total));
                if *total > pt {
                    round(
                        ((*busy as f64 - pb as f64) * 100. / (*total - pt) as f64).clamp(0., 100.),
                        1,
                    )
                } else {
                    0.
                }
            })
            .collect::<Vec<_>>();
        self.cpu_prev = times;
        let disks_now = diskstats();
        let dprev = std::mem::replace(&mut self.disk_prev, (time, disks_now.clone()));
        let net_now = netdev();
        let nprev = std::mem::replace(&mut self.net_prev, (time, net_now.clone()));
        let mut freqs = (0..ncpu())
            .filter_map(|i| {
                int(read(format!(
                    "/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"
                )))
                .map(|n| n as f64 / 1000.)
            })
            .collect::<Vec<_>>();
        if freqs.is_empty() {
            freqs = read("/proc/cpuinfo")
                .lines()
                .filter_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.trim() == "cpu MHz")
                        .and_then(|(_, v)| num(v))
                })
                .collect();
        }
        let st = self.cpuinfo.get_or_insert_with(cpu_static).clone();
        let load = read("/proc/loadavg")
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let total_threads = load
            .get(3)
            .and_then(|s| s.split_once('/'))
            .and_then(|(_, n)| int(n));
        let nprocs = names("/proc")
            .iter()
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .count();
        let uptime = read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(num)
            .unwrap_or(0.);
        let mem = meminfo();
        let m = |key: &str| mem.get(key).copied().unwrap_or(0);
        let total = m("MemTotal");
        let avail = m("MemAvailable");
        let cpu = json!({"model":st["model"],"usage":usage.first().copied().unwrap_or(0.),"perCore":usage.get(1..).unwrap_or(&[]),"freqMHz":if freqs.is_empty(){None}else{Some(round(freqs.iter().sum::<f64>()/freqs.len()as f64,0)as i64)},"cores":st["cores"],"threads":st["threads"],"uptime":uptime as i64,"processes":nprocs,"threadsTotal":total_threads,"temperature":cpu_temperature(),"maxMHz":st["maxMHz"],"baseMHz":st["baseMHz"],"load":if load.len()>=3{Some(load[..3].iter().filter_map(|s|num(s)).collect::<Vec<_>>())}else{None}});
        let memory = json!({"total":total,"used":total.saturating_sub(avail),"available":avail,"cached":m("Cached").saturating_add(m("SReclaimable")).saturating_sub(m("Shmem")),"buffers":m("Buffers"),"swapTotal":m("SwapTotal"),"swapUsed":m("SwapTotal").saturating_sub(m("SwapFree")),"committed":m("Committed_AS"),"commitLimit":m("CommitLimit"),"shared":m("Shmem"),"kernel":m("Slab").saturating_add(m("KernelStack")).saturating_add(m("PageTables"))});
        let mm = mount_map();
        let mut disks = vec![];
        let dt = time - dprev.0;
        for (index, name) in physical_disks().iter().enumerate() {
            let info = disk_info(name);
            let (mut rd, mut wr, mut active, mut response) = (0., 0., 0., None);
            if let (Some(cur), Some(old)) = (disks_now.get(name), dprev.1.get(name)) {
                if dt > 0. {
                    rd = cur[0].saturating_sub(old[0]) as f64 * 512. / dt;
                    wr = cur[1].saturating_sub(old[1]) as f64 * 512. / dt;
                    active = (cur[2].saturating_sub(old[2]) as f64 / (dt * 10.)).min(100.);
                    let ios = cur[3] as i128 - old[3] as i128;
                    response = Some(if ios > 0 {
                        round((cur[4] as f64 - old[4] as f64) / ios as f64, 2)
                    } else {
                        0.
                    });
                }
            }
            disks.push(json!({"name":tr(&lang,"disk_n",&[("n",index.to_string())]),"device":name,"index":index,"model":info["model"],"kind":info["kind"],"readBps":round(rd,0)as u64,"writeBps":round(wr,0)as u64,"activePct":round(active,1),"responseMs":response,"capacity":int(read(format!("/sys/block/{name}/size"))).unwrap_or(0).max(0)as u64*512,"mounts":disk_mounts(name,&mm),"removable":info["removable"]}));
        }
        if time - self.ips.0 >= 10. {
            self.ips = (time, ip_addrs());
        }
        if time - self.primary.0 >= 5. {
            self.primary = (
                time,
                ctx.call("system.network.get", &json!({}))
                    .map(|v| v["primary"].clone())
                    .unwrap_or(Value::Null),
            );
        }
        let mut counts = HashMap::<String, usize>::new();
        let mut network = vec![];
        let dtn = time - nprev.0;
        for (n, kind, slot) in adapters() {
            let (mut rx, mut tx) = (0., 0.);
            if let (Some(cur), Some(old)) = (net_now.get(&n), nprev.1.get(&n)) {
                if dtn > 0. {
                    rx = cur[0].saturating_sub(old[0]) as f64 / dtn;
                    tx = cur[1].saturating_sub(old[1]) as f64 / dtn;
                }
            }
            let count = counts.entry(kind.clone()).or_default();
            *count += 1;
            let label = match (kind.as_str(), *count) {
                ("wifi", 1) => tr(&lang, "wifi", &[]),
                ("wifi", _) => tr(&lang, "wifi_n", &[("n", count.to_string())]),
                ("ethernet", 1) => tr(&lang, "ethernet", &[]),
                ("ethernet", _) => tr(&lang, "ethernet_n", &[("n", count.to_string())]),
                _ => format!("{} ({n})", tr(&lang, "vpn", &[])),
            };
            let ip = self.ips.1.get(&n).cloned().unwrap_or(Value::Null);
            let speed = int(read(format!("/sys/class/net/{n}/speed"))).filter(|n| *n > 0);
            let state = read(format!("/sys/class/net/{n}/operstate"));
            let adapter = if slot.len() >= 5
                && slot.as_bytes()[..4].iter().all(|b| b.is_ascii_hexdigit())
                && slot.as_bytes()[4] == b':'
            {
                pci_name(&slot)
            } else {
                None
            };
            let mut item = json!({"iface":n,"kind":kind,"name":label,"rxBps":round(rx,0)as u64,"txBps":round(tx,0)as u64,"ipv4":ip["ipv4"],"ipv6":ip["ipv6"],"connected":state=="up"||state=="unknown","linkSpeedMbps":speed,"adapter":adapter});
            if self.primary.1.is_object() && self.primary.1["device"] == n {
                item["connection"] = self.primary.1["name"].clone();
            }
            network.push(item);
        }
        json!({"timestamp":(wall()*1000.)as i64,"cpu":cpu,"memory":memory,"disks":disks,"network":network,"gpus":gpu.stats()})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_optional_fields_are_null_without_losing_available_metrics() {
        let data = "GeForce Test, 9, 10, 20, 50, N/A, N/A, 15, 1000, 1.2, 00000000:01:00.0\n";
        let rows = parse_nvidia(data);
        assert_eq!(rows[0]["name"], "NVIDIA GeForce Test");
        assert_eq!(rows[0]["memUsed"], 10485760);
        assert!(rows[0]["engines"]["encode"].is_null());
        assert_eq!(rows[0]["slot"], "0000:01:00.0");
    }
    #[test]
    fn pci_friendly_names_keep_model_and_normalize_vendor() {
        assert_eq!(
            nice_pci("NVIDIA Corporation", "GA102 [GeForce RTX 3080]"),
            "NVIDIA GeForce RTX 3080"
        );
        assert_eq!(
            nice_pci("Intel Corporation", "Intel Device"),
            "Intel Device"
        );
    }
}
pub(super) fn gpu_names(gpu: &Arc<Gpu>) -> Vec<String> {
    let mut names = gpu
        .nvidia(3600., 3.)
        .iter()
        .filter_map(|g| g["name"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let have_nv = !names.is_empty();
    for (_, _, vendor, slot) in drm_cards() {
        if vendor == "nvidia" && have_nv {
            continue;
        }
        if !slot.is_empty() {
            if let Some(n) = pci_name(&slot) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
    }
    if names.is_empty() && command_exists("lspci") {
        let r = run("lspci", &["-mm"], 3.);
        if r.ok {
            for l in r.out.lines() {
                if let Ok(f) = glib::shell_parse_argv(l) {
                    if f.len() > 3
                        && ["VGA", "3D", "Display"]
                            .iter()
                            .any(|s| f[1].to_string_lossy().contains(s))
                    {
                        names.push(nice_pci(&f[2].to_string_lossy(), &f[3].to_string_lossy()));
                    }
                }
            }
        }
    }
    names
}
