use super::{ServiceContext, common::*};
use gio::prelude::*;
use glib::translate::ToGlibPtr;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs,
    path::Path,
    sync::Arc,
};
#[derive(Clone, Debug)]
pub(super) struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub state: String,
    pub ticks: u64,
    pub nice: i64,
    pub threads: u64,
    pub start: u64,
    pub private: u64,
    pub uid: u32,
}
#[derive(Clone, Debug)]
pub(super) struct Static {
    pub cmd: String,
    pub argv: Vec<String>,
    pub exe: Option<String>,
    pub cgroup: String,
    pub unit: Option<Unit>,
    pub user: String,
}
pub(super) fn stat_of(pid: u32) -> Option<(String, Vec<String>)> {
    let data = read_bytes(format!("/proc/{pid}/stat"), 2048)?;
    let lp = data.iter().position(|b| *b == b'(')?;
    let rp = data.iter().rposition(|b| *b == b')')?;
    if rp <= lp {
        return None;
    }
    Some((
        String::from_utf8_lossy(&data[lp + 1..rp]).into_owned(),
        String::from_utf8_lossy(data.get(rp + 2..)?)
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    ))
}
pub(super) fn argv_of(pid: u32) -> Vec<String> {
    read_bytes(format!("/proc/{pid}/cmdline"), 65536)
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect()
}
pub(super) fn scan_proc() -> BTreeMap<u32, Proc> {
    let mut out = BTreeMap::new();
    for n in names("/proc") {
        let Ok(pid) = n.parse::<u32>() else { continue };
        let Some((comm, f)) = stat_of(pid) else {
            continue;
        };
        if f.len() < 20 {
            continue;
        }
        let sm = read(format!("/proc/{pid}/statm"))
            .split_whitespace()
            .filter_map(|s| s.parse::<i64>().ok())
            .collect::<Vec<_>>();
        let Some(uid) = metadata_uid(format!("/proc/{pid}")) else {
            continue;
        };
        let parse = || -> Option<Proc> {
            Some(Proc {
                pid,
                ppid: f[1].parse().ok()?,
                comm,
                state: f[0].clone(),
                ticks: f[11]
                    .parse::<u64>()
                    .ok()?
                    .saturating_add(f[12].parse::<u64>().ok()?),
                nice: f[16].parse().ok()?,
                threads: f[17].parse().ok()?,
                start: f[19].parse().ok()?,
                private: if sm.len() > 2 {
                    ((sm[1] - sm[2]) * page()).max(0) as u64
                } else {
                    0
                },
                uid,
            })
        };
        if let Some(p) = parse() {
            out.insert(pid, p);
        }
    }
    out
}
fn static_of(p: &Proc) -> Static {
    let argv = read_bytes(format!("/proc/{}/cmdline", p.pid), 8192)
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect::<Vec<_>>();
    let cmd = argv.join(" ");
    let cmd = if cmd.chars().count() > 300 {
        format!("{}…", cmd.chars().take(299).collect::<String>())
    } else {
        cmd
    };
    let exe = if p.uid == unsafe { libc::getuid() } {
        fs::read_link(format!("/proc/{}/exe", p.pid))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    let cg = cgroup(p.pid);
    Static {
        cmd,
        argv,
        exe,
        unit: parse_cgroup(&cg),
        cgroup: cg,
        user: username(p.uid),
    }
}
#[derive(Clone, Debug)]
struct Desktop {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub hidden: bool,
    pub terminal: bool,
}
#[derive(Default)]
struct DesktopIndex {
    built: f64,
    lang: String,
    by_id: HashMap<String, Desktop>,
    by_wm: HashMap<String, Desktop>,
    by_exec: HashMap<String, Desktop>,
    by_path: HashMap<String, Desktop>,
    by_directory: HashMap<String, Desktop>,
    ambiguous_exec: HashSet<String>,
    ambiguous_path: HashSet<String>,
    ambiguous_directory: HashSet<String>,
    autostart: HashSet<String>,
}
impl DesktopIndex {
    // Runtime executables may be shared by several desktop entries. Never pick
    // whichever GIO happened to enumerate last (notably Electron/web apps).
    fn put_unique(table: &mut HashMap<String, Desktop>, ambiguous: &mut HashSet<String>, key: String, entry: &Desktop) {
        if key.is_empty() || entry.hidden || entry.terminal || ambiguous.contains(&key) { return; }
        if table.get(&key).is_some_and(|old| old.id != entry.id) {
            table.remove(&key); ambiguous.insert(key);
        } else { table.insert(key, entry.clone()); }
    }
    fn index_path(&mut self, executable: &Path, entry: &Desktop) {
        if !executable.is_absolute() || entry.hidden || entry.terminal { return; }
        let key=executable.to_string_lossy().into_owned();
        Self::put_unique(&mut self.by_path,&mut self.ambiguous_path,key.clone(),entry);
        if let Some(directory)=application_directory(&key) {
            Self::put_unique(&mut self.by_directory,&mut self.ambiguous_directory,directory,entry);
        }
    }
    fn put(table: &mut HashMap<String, Desktop>, key: String, entry: &Desktop) {
        if key.is_empty() {
            return;
        }
        if table
            .get(&key)
            .is_none_or(|old| !entry.hidden && old.hidden)
        {
            table.insert(key, entry.clone());
        }
    }
    fn ensure(&mut self, lang: &str) {
        if self.built > 0. && now() - self.built < 600. && self.lang == lang {
            return;
        }
        self.by_id.clear();
        self.by_wm.clear();
        self.by_exec.clear();
        self.by_path.clear(); self.by_directory.clear();
        self.ambiguous_exec.clear(); self.ambiguous_path.clear(); self.ambiguous_directory.clear();
        self.autostart.clear();
        for a in gio::AppInfo::all() {
            let Some(id) = a
                .id()
                .map(|s| s.to_string())
                .filter(|s| s.ends_with(".desktop"))
            else {
                continue;
            };
            let desktop = a.downcast_ref::<gio::DesktopAppInfo>();
            let mut name = a.name().to_string();
            if name.is_empty() {
                name = id[..id.len() - 8].into()
            }
            let mut raw = BTreeMap::new();
            if let Some(path) = desktop.and_then(|d| d.filename()) {
                raw = super::startup::parse(&read(&path));
                let localized = super::startup::local(&raw, "Name", lang);
                if !localized.is_empty() {
                    name = localized
                }
            }
            let entry = Desktop {
                id: id.clone(),
                name,
                icon: a
                    .icon()
                    .and_then(|i| gio::prelude::IconExt::to_string(&i))
                    .map(|s| s.to_string()),
                hidden: raw.get("NoDisplay").is_some_and(|s| s == "true"),
                terminal: raw.get("Terminal").is_some_and(|s| s == "true"),
            };
            let base = id[..id.len() - 8].to_lowercase();
            Self::put(&mut self.by_id, base.clone(), &entry);
            if let Some(flat) = raw.get("X-Flatpak") {
                Self::put(&mut self.by_id, flat.to_lowercase(), &entry)
            }
            if let Some(snap) = raw.get("X-SnapInstanceName") {
                Self::put(&mut self.by_wm, snap.to_lowercase(), &entry)
            }
            if let Some(wms) = raw.get("StartupWMClass") {
                for wm in wms.split(',') {
                    Self::put(&mut self.by_wm, wm.trim().to_lowercase(), &entry)
                }
            }
            let raw_exe = unsafe { gio::ffi::g_app_info_get_executable(a.to_glib_none().0) };
            let exe = if raw_exe.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(raw_exe) }
                    .to_string_lossy()
                    .into_owned()
            };
            let exebase = basename(exe.trim().trim_matches('"')).to_lowercase();
            if !exebase.is_empty() && !generic(&exebase) && !entry.terminal {
                Self::put_unique(&mut self.by_exec, &mut self.ambiguous_exec, exebase.clone(), &entry);
                if let Some(s) = exebase.strip_suffix("-stable") {
                    Self::put_unique(&mut self.by_exec, &mut self.ambiguous_exec, s.into(), &entry)
                }
                let executable=if Path::new(&exe).is_absolute() { Some(Path::new(&exe).to_owned()) } else { glib::find_program_in_path(&exe) };
                if let Some(path)=executable {
                    self.index_path(&path,&entry);
                    if let Ok(real)=fs::canonicalize(path) { self.index_path(&real,&entry); }
                }
            }
            if base.contains('.') {
                Self::put_unique(
                    &mut self.by_exec, &mut self.ambiguous_exec,
                    base.rsplit('.').next().unwrap_or("").into(),
                    &entry,
                )
            }
        }
        for d in [
            Path::new("/etc/xdg/autostart").to_owned(),
            config_home().join("autostart"),
        ] {
            for f in names(d) {
                if let Some(stem) = f.strip_suffix(".desktop") {
                    self.autostart.insert(stem.to_lowercase());
                }
            }
        }
        for (alias, target) in [
            ("chrome", "google-chrome"),
            ("brave", "brave-browser"),
            ("code", "code"),
            ("soffice.bin", "libreoffice-startcenter"),
        ] {
            if !self.by_exec.contains_key(alias) && !self.ambiguous_exec.contains(alias) {
                if let Some(e) = self.by_id.get(target) {
                    self.by_exec.insert(alias.into(), e.clone());
                }
            }
        }
        self.built = now();
        self.lang = lang.into();
    }
    fn lookup(&self, key: &str) -> Option<Desktop> {
        let mut k = key.to_lowercase();
        let get = |k: &str| self.by_id.get(k).or_else(|| self.by_wm.get(k)).cloned();
        let mut hit = get(&k);
        if hit.is_none() {
            if let Some(s) = k.strip_suffix(".desktop") {
                k = s.into();
                hit = get(&k)
            }
        }
        if hit.is_none() && k.contains('.') {
            let last = k.rsplit('.').next().unwrap_or("");
            hit = self
                .by_wm
                .get(last)
                .or_else(|| self.by_id.get(last))
                .or_else(|| self.by_exec.get(last))
                .cloned();
        }
        hit
    }
    fn executable(&self, name: &str) -> Option<Desktop> {
        self.by_exec
            .get(&name.to_lowercase())
            .filter(|e| !e.hidden)
            .cloned()
    }
    fn process(&self, st: &Static, comm: &str) -> Option<Desktop> {
        let path=st.exe.as_deref().unwrap_or("").trim_end_matches(" (deleted)");
        let name=basename(path).to_lowercase();
        if !path.is_empty() && !generic(&name) {
            if let Some(entry)=self.by_path.get(path) { return Some(entry.clone()); }
            // A reparented crash reporter/helper can outlive its GUI. A unique,
            // app-specific installation directory remains a durable identity;
            // shared system bins and shared runtime directories are excluded.
            if let Some(entry)=self.by_directory.iter()
                .filter(|(directory,_)| Path::new(path).starts_with(directory.as_str()))
                .max_by_key(|(directory,_)| directory.len()).map(|(_,entry)|entry) {
                return Some(entry.clone());
            }
            if let Some(entry)=self.executable(&name) { return Some(entry); }
        }
        if !generic(&comm.to_lowercase()) { self.executable(comm) } else { None }
    }
    fn window(&self, st:&Static, window:&Value) -> Option<Desktop> {
        // An installed window identity may intentionally launch a wrapper or
        // share a runtime with another app. Executable mismatch alone cannot
        // invalidate it. Only raw app-ID hints and scope IDs are weaker.
        if let Some(entry)=self.lookup(window["desktopId"].as_str().unwrap_or("")) {return Some(entry);}
        let path=st.exe.as_deref().unwrap_or("").trim_end_matches(" (deleted)");
        self.by_path.get(path).filter(|_|!generic(&basename(path).to_lowercase())).cloned()
            .or_else(||self.lookup(window["appId"].as_str().unwrap_or("")))
    }
}

fn application_directory(executable: &str) -> Option<String> {
    fn stem(name: &str) -> String {
        let mut name=name.to_lowercase();
        for _ in 0..2 {
            let Some(suffix)=[".appimage",".appdir",".bin","-bin","-stable"].into_iter().find(|suffix|name.ends_with(*suffix)) else {break};
            name.truncate(name.len()-suffix.len());
        }
        name
    }
    let path=Path::new(executable);
    let mut directory=path.parent()?;
    if directory.file_name().is_some_and(|part|part=="bin"||part=="lib"||part=="libexec") {
        directory=directory.parent()?;
    }
    if !directory.is_absolute() { return None; }
    // A sole desktop entry is not evidence that a shared directory belongs to
    // that app. Require a positive app-named installation boundary as well.
    if stem(directory.file_name()?.to_str()?)!=stem(path.file_name()?.to_str()?) {return None;}
    let text=directory.to_string_lossy();
    if ["/","/bin","/sbin","/usr/bin","/usr/sbin","/usr/local/bin","/usr/local/sbin",
        "/lib","/lib64","/usr/lib","/usr/lib64","/usr/local/lib","/opt","/app","/app/bin","/app/lib","/snap/bin","/tmp","/var/tmp"]
        .contains(&text.as_ref()) { return None; }
    if let Some(home)=std::env::var_os("HOME") {
        let home=Path::new(&home);
        if directory==home || [".local",".local/bin",".cargo/bin","Downloads","Documents","Desktop"]
            .iter().any(|part| directory==home.join(part)) { return None; }
    }
    Some(text.into_owned())
}

fn external_unit(unit: &Unit) -> bool {
    unit.name.starts_with("app-win11-external-") && unit.name.ends_with(".service")
}
#[derive(Default)]
pub(super) struct ProcessState {
    pub statics: HashMap<(u32, u64), Static>,
    samples: VecDeque<(f64, BTreeMap<u32, (u64, u64)>)>,
    io_prev: HashMap<(u32, u64), (f64, u64, u64)>,
    disk_prev: Option<(f64, BTreeMap<String, [u64; 5]>)>,
    net_prev: Option<(f64, BTreeMap<String, [u64; 2]>)>,
    high_since: HashMap<(u32, u64), f64>,
    last_cpu: HashMap<(u32, u64), f64>,
    last_names: HashMap<(u32, u64), (String, String)>,
    /// The language `last_names` was built in (the health page shows those names later).
    names_lang: String,
    cpu_prev: Option<(u64, u64)>,
    pub weights: (f64, HashMap<String, Option<i64>>),
    windows: (f64, Vec<Value>),
    units_desc: (f64, HashMap<String, String>),
    pub critical: HashSet<u32>,
    crit_time: f64,
    desktop: DesktopIndex,
}
impl ProcessState {
    pub fn record(
        &mut self,
        time: f64,
        ticks: BTreeMap<u32, (u64, u64)>,
    ) -> HashMap<(u32, u64), f64> {
        let base = self
            .samples
            .iter()
            .rev()
            .find(|(t, _)| time - t >= 0.4)
            .or_else(|| self.samples.front());
        let mut cpu = HashMap::new();
        if let Some((t, prev)) = base {
            let dt = time - t;
            if dt > 0. {
                let scale = 100. / (dt * clk() * ncpu() as f64);
                for (pid, (start, t)) in &ticks {
                    if let Some((s, p)) = prev.get(pid) {
                        if s == start && t > p {
                            cpu.insert((*pid, *start), (t - p) as f64 * scale);
                        }
                    }
                }
            }
        }
        let has_base = base.is_some();
        self.samples.push_back((time, ticks));
        while self.samples.len() > 4 {
            self.samples.pop_front();
        }
        let threshold = 80. / ncpu() as f64;
        for (key, c) in &cpu {
            if *c >= threshold {
                self.high_since.entry(*key).or_insert(time);
            } else {
                self.high_since.remove(key);
            }
        }
        if has_base {
            self.high_since.retain(|k, _| cpu.contains_key(k));
        }
        self.last_cpu = cpu.clone();
        cpu
    }
    pub fn sample(&mut self) {
        let mut ticks = BTreeMap::new();
        for name in names("/proc") {
            let Ok(pid) = name.parse() else { continue };
            if let Some((_, f)) = stat_of(pid) {
                if f.len() > 19 {
                    if let (Ok(start), Ok(a), Ok(b)) = (
                        f[19].parse::<u64>(),
                        f[11].parse::<u64>(),
                        f[12].parse::<u64>(),
                    ) {
                        ticks.insert(pid, (start, a.saturating_add(b)));
                    }
                }
            }
        }
        self.record(now(), ticks);
    }
    /// The language of the names [`Self::high_cpu`] gives ("" before the first process list).
    pub fn names_lang(&self) -> &str {
        &self.names_lang
    }
    pub fn high_cpu(&self, seconds: f64) -> Vec<Value> {
        let time = now();
        let mut out=self.high_since.iter().filter(|(_,since)|time-**since>=seconds).map(|((pid,start),since)|{let name=self.last_names.get(&(*pid,*start));json!({"pid":pid,"name":name.map(|v|v.0.clone()).unwrap_or_else(||{let s=read(format!("/proc/{pid}/comm"));if s.is_empty(){"?".into()}else{s}}),"since":time-since,"cpu":self.last_cpu.get(&(*pid,*start)).copied().unwrap_or(0.),"groupKey":name.map(|n|n.1.clone())})}).collect::<Vec<_>>();
        out.sort_by(|a, b| {
            b["cpu"]
                .as_f64()
                .unwrap_or(0.)
                .total_cmp(&a["cpu"].as_f64().unwrap_or(0.))
        });
        out
    }
    pub fn refresh_critical(&mut self, procs: &BTreeMap<u32, Proc>) {
        let mut crit = HashSet::from([1, std::process::id()]);
        let mut p = unsafe { libc::getppid() } as u32;
        for _ in 0..64 {
            if p <= 1 {
                break;
            }
            crit.insert(p);
            p = procs.get(&p).map(|p| p.ppid).unwrap_or(0);
        }
        for (pid, p) in procs {
            if is_critical_comm(&p.comm) {
                crit.insert(*pid);
            }
        }
        self.critical = crit;
        self.crit_time = now();
    }
    pub fn descriptions(&mut self) -> HashMap<String, String> {
        if now() - self.units_desc.0 < 300. && !self.units_desc.1.is_empty() {
            return self.units_desc.1.clone();
        }
        let out = list_units(None)
            .into_iter()
            .filter_map(|v| {
                Some((
                    v["unit"].as_str()?.into(),
                    v["description"].as_str().unwrap_or("").into(),
                ))
            })
            .collect();
        self.units_desc = (now(), out);
        self.units_desc.1.clone()
    }
    pub fn processes(&mut self, ctx: &Arc<ServiceContext>, include: bool) -> Value {
        let time = now();
        let procs = scan_proc();
        let lang = ctx.language();
        self.desktop.ensure(&lang);
        if time - self.windows.0 >= 0.8 {
            let wins = ctx
                .call("windows.list", &json!({}))
                .ok()
                .filter(|v| truth(&v["supported"]))
                .and_then(|v| v["windows"].as_array().cloned())
                .unwrap_or_default();
            self.windows = (time, wins);
        }
        let windows = self.windows.1.clone();
        let cpu = self.record(
            time,
            procs
                .iter()
                .map(|(pid, p)| (*pid, (p.start, p.ticks)))
                .collect(),
        );
        let live = procs
            .iter()
            .map(|(pid, p)| (*pid, p.start))
            .collect::<HashSet<_>>();
        for (pid, p) in &procs {
            self.statics
                .entry((*pid, p.start))
                .or_insert_with(|| static_of(p));
        }
        self.statics.retain(|k, _| live.contains(k));
        let mut io_rate = HashMap::new();
        let mut new_io = HashMap::new();
        for (pid, p) in &procs {
            if p.uid != unsafe { libc::getuid() } || p.comm == "kthreadd" {
                continue;
            }
            let data = read(format!("/proc/{pid}/io"));
            if data.is_empty() {
                continue;
            }
            let field = |name: &str| {
                data.lines()
                    .find_map(|l| {
                        l.strip_prefix(name)
                            .and_then(|s| s.trim().parse::<u64>().ok())
                    })
                    .unwrap_or(0)
            };
            let rd = field("read_bytes:");
            let wr = field("write_bytes:");
            let key = (*pid, p.start);
            new_io.insert(key, (time, rd, wr));
            if let Some((t, r, w)) = self.io_prev.get(&key) {
                if time > *t {
                    io_rate.insert(
                        *pid,
                        (
                            rd.saturating_sub(*r) as f64 / (time - t),
                            wr.saturating_sub(*w) as f64 / (time - t),
                        ),
                    );
                }
            }
        }
        self.io_prev = new_io;
        if time - self.crit_time > 10. || self.critical.is_empty() {
            self.refresh_critical(&procs)
        }
        let uid = unsafe { libc::getuid() };
        let mypid = std::process::id();
        let mut app_of = HashMap::<u32, String>::new();
        let mut meta = HashMap::<String, Value>::new();
        let mut win_by_pid = BTreeMap::<u32, Vec<Value>>::new();
        for w in windows {
            if let Some(pid) = valint(&w["pid"]).and_then(|v| u32::try_from(v).ok()) {
                win_by_pid.entry(pid).or_default().push(w);
            }
        }
        let mut app_meta =
            |key: String, name: String, icon: Option<String>, desktop: Option<String>| {
                meta.entry(key.clone())
                    .or_insert_with(|| json!({"name":name,"icon":icon,"desktopId":desktop}));
                key
            };
        // The own apps' windows first, then the processes below them in their unit (own_window_of: a native app's
        // backend and its workers), then everything else.
        let mut own_windows = HashSet::new();
        for (pid, p) in &procs {
            if p.uid != uid || *pid == mypid {
                continue;
            }
            let st = &self.statics[&(*pid, p.start)];
            let parent = procs
                .get(&p.ppid)
                .and_then(|q| self.statics.get(&(p.ppid, q.start)))
                .map_or(&[][..], |s| s.argv.as_slice());
            if let Some(own) = own_app(&st.argv, parent) {
                let (name, icon) = own_names(&lang, &own);
                let key = format!("win11shell-{own}.desktop");
                app_of.insert(
                    *pid,
                    app_meta(key.clone(), name, Some(icon.into()), Some(key)),
                );
                own_windows.insert(*pid);
            }
        }
        let helpers = procs
            .iter()
            .filter(|(pid, p)| p.uid == uid && !app_of.contains_key(pid))
            .filter_map(|(pid, _)| {
                own_window_of(
                    *pid,
                    &procs,
                    &self.statics,
                    &app_of,
                    &own_windows,
                    &win_by_pid,
                    mypid,
                )
                .map(|key| (*pid, key))
            })
            .collect::<Vec<_>>();
        app_of.extend(helpers);
        let mut confirmed_apps=app_of.keys().copied().collect::<HashSet<_>>();
        for (pid, p) in &procs {
            if p.uid != uid || *pid == mypid || app_of.contains_key(pid) {
                continue;
            }
            let st = &self.statics[&(*pid, p.start)];
            // A scope can name a generic runtime. A real window is stronger
            // than the executable (shared runtimes/PWAs), and both beat scopes.
            if let Some(wins) = win_by_pid.get(pid) {
                let w = &wins[0];
                let entry = self.desktop.window(st,w);
                let key = if let Some(e) = entry {
                    app_meta(e.id.clone(), e.name, e.icon, Some(e.id))
                } else {
                    let aid = w["appId"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&p.comm);
                    app_meta(
                        format!("win:{aid}"),
                        aid.into(),
                        Some(
                            w["icon"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(aid)
                                .into(),
                        ),
                        w["desktopId"].as_str().map(str::to_owned),
                    )
                };
                app_of.insert(*pid, key);
                confirmed_apps.insert(*pid);continue;
            }
            if let Some(e)=self.desktop.process(st,&p.comm) {
                app_of.insert(*pid,app_meta(e.id.clone(),e.name,e.icon,Some(e.id)));
                confirmed_apps.insert(*pid);continue;
            }
            if let Some(unit)=st.unit.as_ref().filter(|unit|unit.kind=="scope") {
                if let Some(e)=self.desktop.lookup(&unit.id) {
                    if !e.hidden {app_of.insert(*pid,app_meta(e.id.clone(),e.name,e.icon,Some(e.id)));}
                } else if !self.desktop.autostart.contains(&unit.id.to_lowercase()) {
                    let pretty=unit.id.rsplit('.').next().unwrap_or(&unit.id);
                    let mut chars=pretty.chars();
                    let name=chars.next().map(|c|c.to_uppercase().collect::<String>()+chars.as_str()).unwrap_or_default();
                    app_of.insert(*pid,app_meta(format!("scope:{}",unit.id),name,Some(unit.id.clone()),None));
                }
            }
        }
        drop(app_meta);
        // An unregistered app still has a strong executable identity once its
        // window is known. Apply that identity to workers in other launcher
        // services and reparented helpers, without inheriting a generic scope.
        let identities=runtime_app_identities(&app_of,&confirmed_apps,&procs,&self.statics,&meta);
        for (pid,p) in &procs {
            if p.uid!=uid || *pid==mypid || confirmed_apps.contains(pid) || self.critical.contains(pid) {continue;}
            if let Some(entry)=identities.process(&self.statics[&(*pid,p.start)],&p.comm) {
                app_of.insert(*pid,entry.id);
            }
        }
        let application_services = application_services(&app_of, &procs, &self.statics);
        let mut memo = HashMap::new();
        for (pid, p) in &procs {
            if p.uid == uid && !app_of.contains_key(pid) && *pid != mypid {
                if let Some(key) = resolve(
                    *pid,
                    0,
                    &app_of,
                    &application_services,
                    &mut memo,
                    &procs,
                    &self.statics,
                    &self.critical,
                    &self.desktop,
                    &mut meta,
                ) {
                    app_of.insert(*pid, key);
                }
            }
        }
        let mut host_children = HashSet::new();
        for (pid, p) in &procs {
            if p.uid == uid && !app_of.contains_key(pid) {
                let mut q = p.ppid;
                for _ in 0..32 {
                    if q <= 1 {
                        break;
                    }
                    if q == mypid {
                        host_children.insert(*pid);
                        break;
                    }
                    if app_of.contains_key(&q) {
                        break;
                    }
                    q = procs.get(&q).map(|p| p.ppid).unwrap_or(0);
                }
            }
        }
        let descriptions = self.descriptions();
        if time - self.weights.0 > 5. {
            self.weights = (time, HashMap::new());
        }
        for (pid, p) in &procs {
            let st = &self.statics[&(*pid, p.start)];
            if p.uid == uid && st.unit.is_some() {
                self.weights.1.entry(st.cgroup.clone()).or_insert_with(|| {
                    int(read(format!("/sys/fs/cgroup{}/cpu.weight", st.cgroup)))
                });
            }
        }
        let mut groups = HashMap::<String, Group>::new();
        let mut details = vec![];
        let mut threads = 0;
        for (pid, p) in &procs {
            threads += p.threads;
            let key = (*pid, p.start);
            let st = &self.statics[&key];
            let c = round(cpu.get(&key).copied().unwrap_or(0.), 2);
            let (rd, wr) = io_rate.get(pid).copied().unwrap_or((0., 0.));
            let kernel = *pid == 2 || p.ppid == 2;
            let (group, gkey, name, icon, desk) = if (*pid == mypid || host_children.contains(pid)) && !app_of.contains_key(pid) {
                (
                    "background",
                    "win11shell".into(),
                    tr(&lang, "host", &[]),
                    Some("preferences-desktop".into()),
                    None,
                )
            } else if p.uid == uid && app_of.contains_key(pid) {
                let key = app_of[pid].clone();
                let m = &meta[&key];
                (
                    "app",
                    key,
                    m["name"].as_str().unwrap_or("").into(),
                    m["icon"].as_str().map(str::to_owned),
                    m["desktopId"].as_str().map(str::to_owned),
                )
            } else if p.uid == uid && !kernel {
                let label = exe_label(st, &p.comm);
                let (key, name) = if st.unit.as_ref().is_some_and(external_unit) {
                    // Old launcher units retain their original Description.
                    // Never present their apps as WinPoint-owned components or
                    // lump unrelated unknown executables into one service row.
                    let identity=st.exe.as_deref().unwrap_or("").trim_end_matches(" (deleted)");
                    let key=if identity.is_empty()||generic(&basename(identity).to_lowercase()) {
                        format!("external-pid:{pid}:{}",p.start)
                    } else {format!("external-exe:{identity}")};
                    let name=if label.is_empty()||label=="?" {
                        crate::i18n::tr(&lang,"sysmon.externalApps")
                    } else {friendly(&lang,&[&label,&p.comm]).unwrap_or_else(||label.clone())};
                    (key,name)
                } else if let Some(u) = st.unit.as_ref().filter(|u| {
                    (u.kind == "service" || u.kind == "unit") && u.name.ends_with(".service")
                }) {
                    (
                        format!("unit:{}", u.name),
                        unit_friendly(&lang, &u.name)
                            .or_else(|| friendly(&lang, &[&label, &p.comm]))
                            .or_else(|| {
                                descriptions
                                    .get(&u.name)
                                    .filter(|s| !s.trim().is_empty())
                                    .map(|s| s.trim().to_owned())
                            })
                            .unwrap_or_else(|| label.clone()),
                    )
                } else if let Some(u) = st.unit.as_ref().filter(|u| u.kind == "scope") {
                    (
                        format!("scope:{}", u.id),
                        unit_friendly(&lang, &u.id)
                            .or_else(|| friendly(&lang, &[&label, &p.comm]))
                            .or_else(|| self.desktop.lookup(&u.id).map(|e| e.name))
                            .unwrap_or_else(|| label.clone()),
                    )
                } else {
                    (
                        format!("exe:{label}"),
                        friendly(&lang, &[&label, &p.comm]).unwrap_or_else(|| label.clone()),
                    )
                };
                let entry = if key.starts_with("exe:") {
                    self.desktop.executable(&p.comm)
                } else {
                    st.unit
                        .as_ref()
                        .filter(|u| u.kind == "scope")
                        .and_then(|u| self.desktop.lookup(&u.id))
                };
                (
                    "background",
                    key,
                    name,
                    entry.as_ref().and_then(|e| e.icon.clone()),
                    entry.map(|e| e.id),
                )
            } else {
                let label = exe_label(st, &p.comm);
                (
                    "system",
                    if kernel {
                        "kernel".into()
                    } else {
                        format!("exe:{label}")
                    },
                    if kernel {
                        tr(&lang, "kernel", &[])
                    } else {
                        friendly(&lang, &[&label, &p.comm]).unwrap_or(label)
                    },
                    None,
                    None,
                )
            };
            let display = if group == "app" {
                name.clone()
            } else {
                friendly(
                    &lang,
                    &[st.exe.as_ref().map(|s| basename(s)).unwrap_or(""), &p.comm],
                )
                .unwrap_or_else(|| name.clone())
            };
            let eff = p.nice >= 10
                || p.uid == uid
                    && st.unit.is_some()
                    && self
                        .weights
                        .1
                        .get(&st.cgroup)
                        .copied()
                        .flatten()
                        .is_some_and(|w| w < 100);
            let can_kill = p.uid == uid && !kernel && !self.critical.contains(pid);
            let g = groups.entry(gkey.clone()).or_insert_with(|| {
                Group::new(
                    gkey.clone(),
                    name,
                    icon,
                    desk,
                    group.into(),
                    st.user.clone(),
                )
            });
            g.pids.push(*pid);
            g.cpu += c;
            g.memory += p.private;
            g.disk += rd + wr;
            g.threads += p.threads;
            g.states.insert(p.state.clone());
            g.eff &= eff;
            g.kill |= can_kill;
            if let Some(wins) = win_by_pid.get(pid) {
                for w in wins {
                    if let Some(title) = w["title"].as_str().filter(|s| !s.is_empty()) {
                        if !g.windows.iter().any(|v| v == title) {
                            g.windows.push(title.into())
                        }
                    }
                }
            }
            if include {
                details.push(json!({"pid":pid,"ppid":p.ppid,"name":p.comm,"displayName":display,"cmd":st.cmd,"user":st.user,"state":state_name(&p.state),"cpu":c,"memory":p.private,"diskRead":round(rd,0)as u64,"diskWrite":round(wr,0)as u64,"threads":p.threads,"started":((boot()+p.start as f64/clk())*1000.)as i64,"group":group,"groupKey":gkey,"canKill":can_kill,"efficiency":eff,"nice":p.nice,"exe":st.exe,"critical":self.critical.contains(pid)}));
            }
            self.last_names.insert(key, (display, gkey));
        }
        self.last_names.retain(|k, _| live.contains(k));
        self.names_lang = lang;
        let mut apps = vec![];
        let mut background = vec![];
        let mut system = vec![];
        for (_, g) in groups {
            let group = g.group.clone();
            let v = g.value();
            match group.as_str() {
                "app" => apps.push(v),
                "background" => background.push(v),
                _ => system.push(v),
            }
        }
        for list in [&mut apps, &mut background, &mut system] {
            list.sort_by_key(|g| g["name"].as_str().unwrap_or("").to_lowercase());
        }
        let mut totals = self.totals(time);
        totals["processes"] = json!(procs.len());
        totals["threads"] = json!(threads);
        let mut result = json!({"timestamp":(wall()*1000.)as i64,"totals":totals,"apps":apps,"background":background,"system":system});
        if include {
            result["processes"] = json!(details)
        }
        result
    }
    fn totals(&mut self, time: f64) -> Value {
        let times = super::performance::cpu_times();
        let mut cpu = 0.;
        if let Some((busy, total)) = times.first().copied() {
            if let Some((pb, pt)) = self.cpu_prev {
                if total > pt {
                    cpu = (busy as f64 - pb as f64) * 100. / (total - pt) as f64;
                }
            }
            self.cpu_prev = Some((busy, total));
        }
        let disks = super::performance::diskstats();
        let physical = super::performance::physical_disks();
        let (mut rd, mut wr, mut active) = (0f64, 0f64, 0f64);
        if let Some((t, prev)) = &self.disk_prev {
            let dt = time - t;
            if dt > 0. {
                for name in physical {
                    if let (Some(a), Some(b)) = (disks.get(&name), prev.get(&name)) {
                        rd += a[0].saturating_sub(b[0]) as f64 * 512. / dt;
                        wr += a[1].saturating_sub(b[1]) as f64 * 512. / dt;
                        active =
                            active.max((a[2].saturating_sub(b[2]) as f64 / (dt * 10.)).min(100.));
                    }
                }
            }
        }
        self.disk_prev = Some((time, disks));
        let net = super::performance::netdev();
        let (mut rx, mut tx) = (0., 0.);
        if let Some((t, prev)) = &self.net_prev {
            let dt = time - t;
            if dt > 0. {
                for (name, a) in &net {
                    if ["lo", "veth", "docker", "br-", "virbr"]
                        .iter()
                        .any(|s| name.starts_with(s))
                    {
                        continue;
                    }
                    if let Some(b) = prev.get(name) {
                        rx += a[0].saturating_sub(b[0]) as f64 / dt;
                        tx += a[1].saturating_sub(b[1]) as f64 / dt;
                    }
                }
            }
        }
        self.net_prev = Some((time, net));
        let mem = super::performance::meminfo();
        let total = mem.get("MemTotal").copied().unwrap_or(0);
        let available = mem.get("MemAvailable").copied().unwrap_or(0);
        json!({"cpu":round(cpu.clamp(0.,100.),1),"memoryUsed":total.saturating_sub(available),"memoryTotal":total,"processes":0,"threads":0,"diskRead":round(rd,0)as u64,"diskWrite":round(wr,0)as u64,"diskActive":round(active,1),"networkRx":round(rx,0)as u64,"networkTx":round(tx,0)as u64,"cpuCount":ncpu()})
    }
}
/// A dedicated application service owns its helpers even after they are reparented. Several window-bearing
/// apps in one service are ambiguous: their descendants must follow their own parent tree instead.
fn runtime_app_identities(
    app_of: &HashMap<u32,String>,
    confirmed: &HashSet<u32>,
    procs: &BTreeMap<u32,Proc>,
    statics: &HashMap<(u32,u64),Static>,
    meta: &HashMap<String,Value>,
) -> DesktopIndex {
    let uid=unsafe{libc::getuid()};
    let mut index=DesktopIndex::default();
    for pid in confirmed {
        let Some(p)=procs.get(pid).filter(|p|p.uid==uid) else {continue};
        let Some(key)=app_of.get(pid) else {continue};
        let Some(st)=statics.get(&(*pid,p.start)) else {continue};
        let Some(path)=st.exe.as_deref() else {continue};
        let path=path.trim_end_matches(" (deleted)");
        if generic(&basename(path).to_lowercase()) {continue;}
        let Some(value)=meta.get(key) else {continue};
        let entry=Desktop{id:key.clone(),name:value["name"].as_str().unwrap_or(key).into(),
            icon:value["icon"].as_str().map(str::to_owned),hidden:false,terminal:false};
        index.index_path(Path::new(path),&entry);
    }
    index
}

fn application_services(
    app_of: &HashMap<u32, String>,
    procs: &BTreeMap<u32, Proc>,
    statics: &HashMap<(u32, u64), Static>,
) -> HashMap<String, Option<String>> {
    let uid = unsafe { libc::getuid() };
    let mut owners = HashMap::<String, Option<String>>::new();
    for (pid, app) in app_of {
        let Some(p) = procs.get(pid).filter(|p| p.uid == uid) else { continue };
        let Some(st) = statics.get(&(*pid, p.start)) else { continue };
        if !st.unit.as_ref().is_some_and(|u| u.name.ends_with(".service") && !u.name.starts_with("session-")) {
            continue;
        }
        owners.entry(st.cgroup.clone())
            .and_modify(|owner| { if owner.as_ref() != Some(app) { *owner = None; } })
            .or_insert_with(|| Some(app.clone()));
    }
    owners
}

fn resolve(
    pid: u32,
    depth: usize,
    app_of: &HashMap<u32, String>,
    application_services: &HashMap<String, Option<String>>,
    memo: &mut HashMap<u32, Option<String>>,
    procs: &BTreeMap<u32, Proc>,
    statics: &HashMap<(u32, u64), Static>,
    critical: &HashSet<u32>,
    desktop: &DesktopIndex,
    meta: &mut HashMap<String, Value>,
) -> Option<String> {
    if let Some(k) = app_of.get(&pid) {
        return Some(k.clone());
    }
    if let Some(v) = memo.get(&pid) {
        return v.clone();
    }
    let p = procs.get(&pid)?;
    if p.uid != unsafe { libc::getuid() } || pid == std::process::id() || depth > 64 {
        return None;
    }
    let st = &statics[&(pid, p.start)];
    let service_boundary = st.unit.as_ref().is_some_and(|u| {
        (u.kind == "service" || u.kind == "unit") && !u.name.starts_with("session-")
    });
    if service_boundary {
        match application_services.get(&st.cgroup) {
            Some(Some(app)) if !critical.contains(&pid) => {
                memo.insert(pid, Some(app.clone()));
                return Some(app.clone());
            }
            Some(None) if !critical.contains(&pid) => {} // shared service: follow only this app's ancestors
            _ => {
                memo.insert(pid, None);
                return None;
            }
        }
    }
    let mut res = None;
    let same_service = !service_boundary || procs.get(&p.ppid)
        .and_then(|parent| statics.get(&(parent.pid, parent.start)))
        .is_some_and(|parent| parent.cgroup == st.cgroup);
    let known_parent = app_of.contains_key(&p.ppid);
    if same_service && p.ppid > 1 && (p.ppid != std::process::id() || known_parent)
        && (!critical.contains(&p.ppid) || known_parent) {
        res = resolve(
            p.ppid,
            depth + 1,
            app_of,
            application_services,
            memo,
            procs,
            statics,
            critical,
            desktop,
            meta,
        );
    }
    if res.is_none()
        && st
            .unit
            .as_ref()
            .is_none_or(|u| u.name.starts_with("session-") || u.name.starts_with("init.scope"))
    {
        if let Some(e) = desktop
            .executable(st.exe.as_ref().map(|s| basename(s)).unwrap_or(&p.comm))
            .or_else(|| desktop.executable(&p.comm))
        {
            if !is_critical_comm(&p.comm) {
                meta.entry(e.id.clone())
                    .or_insert_with(|| json!({"name":e.name,"icon":e.icon,"desktopId":e.id}));
                res = Some(e.id);
            }
        }
    }
    memo.insert(pid, res.clone());
    res
}
struct Group {
    key: String,
    name: String,
    icon: Option<String>,
    desk: Option<String>,
    group: String,
    user: String,
    pids: Vec<u32>,
    cpu: f64,
    memory: u64,
    disk: f64,
    threads: u64,
    states: BTreeSet<String>,
    eff: bool,
    kill: bool,
    windows: Vec<String>,
}
impl Group {
    fn new(
        key: String,
        name: String,
        icon: Option<String>,
        desk: Option<String>,
        group: String,
        user: String,
    ) -> Self {
        Self {
            key,
            name,
            icon,
            desk,
            group,
            user,
            pids: vec![],
            cpu: 0.,
            memory: 0,
            disk: 0.,
            threads: 0,
            states: BTreeSet::new(),
            eff: true,
            kill: false,
            windows: vec![],
        }
    }
    fn value(mut self) -> Value {
        self.pids.sort_unstable();
        json!({"key":self.key,"name":self.name,"icon":self.icon,"desktopId":self.desk,"pids":self.pids,"cpu":round(self.cpu.min(100.),2),"memory":self.memory,"diskRate":round(self.disk,0)as u64,"windows":self.windows,"status":if !self.states.is_empty()&&self.states.iter().all(|s|s=="T"||s=="t"){"suspended"}else if self.eff{"efficiency"}else{"running"},"threads":self.threads,"user":self.user,"canKill":self.kill,"efficiency":self.eff})
    }
}
pub(super) fn exe_label(st: &Static, comm: &str) -> String {
    let name = st
        .exe
        .as_ref()
        .map(|s| basename(s).replace(" (deleted)", ""))
        .unwrap_or_else(|| comm.into());
    if interpreter(&name) && st.argv.len() > 1 {
        for a in &st.argv[1..] {
            if a.starts_with('-') {
                continue;
            }
            let base = basename(a);
            if !base.is_empty() && !base.starts_with('-') {
                return base.chars().take(60).collect();
            }
            break;
        }
    }
    if name.is_empty() { comm.into() } else { name }
}
/// The seven own apps (host launch.rs OWN_APPS); Task Manager shows each as the app `win11shell-<kind>.desktop`.
const OWN_KINDS: [&str; 7] = [
    "settings",
    "taskmgr",
    "explorer",
    "alles",
    "disks",
    "defaultapps",
    "installer",
];
/// The own app whose window a process is, from its argv and its parent's (empty when unknown). Task Manager shows
/// it as that app and restarts it through apps.launch (actions.rs own_window).
/// - The native app (native/apps): `win11-gtk-<kind>`, which `win11-app --app <kind>`, `win11-explorer`,
///   `win11-default-apps` and `win11-installer` exec with the same pid (host lib.rs exec_native_app).
/// - One of those launchers in the moment before its exec (the same pid, so the same app).
///
/// Never an app of its own: a backend (`--http-only`; a native app's `win11-app --app <kind> --http-only` runs
/// below its window and is counted under it, own_window_of), and the portal's file picker (`win11-gtk-explorer
/// --picker-stdio`, an Explorer with picker params, or any Explorer that win11-file-portal started), which must not
/// be shown, ended or restarted as Explorer.
pub(super) fn own_app(argv: &[String], parent: &[String]) -> Option<String> {
    if argv.iter().any(|s| s == "--http-only") {
        return None;
    }
    let program = basename(argv.first()?);
    let kind = if let Some(kind) = program.strip_prefix("win11-gtk-") {
        kind
    } else if program == "win11-app" {
        let i = argv.iter().position(|s| s == "--app")?;
        argv.get(i + 1)?.as_str()
    } else {
        match program {
            "win11-explorer" => "explorer",
            "win11-default-apps" => "defaultapps",
            "win11-installer" => "installer",
            _ => return None,
        }
    };
    if !OWN_KINDS.contains(&kind) || kind == "explorer" && picker(argv, parent) {
        return None;
    }
    Some(kind.into())
}
/// An Explorer process that is the portal's file picker (host picker.rs show_native, native/apps/explorer
/// picker.rs prepare): `--picker-stdio`, `--params` with picker options, or started by win11-file-portal.
fn picker(argv: &[String], parent: &[String]) -> bool {
    let options = |raw: &str| {
        serde_json::from_str::<Value>(raw).is_ok_and(|v| match &v["picker"] {
            Value::Object(_) => true,
            Value::String(s) => serde_json::from_str::<Value>(s).is_ok_and(|p| p.is_object()),
            _ => false,
        })
    };
    argv.iter().any(|s| s == "--picker-stdio")
        || parent
            .first()
            .is_some_and(|s| basename(s) == "win11-file-portal")
        || argv
            .windows(2)
            .any(|w| w[0] == "--params" && options(&w[1]))
}
/// The own app window `pid` runs below inside the same cgroup, as its app key (`app_of` of a pid in `own`). An own
/// app's processes share its systemd unit, which resolve() does not walk out of. The walk ends at a cgroup
/// boundary, at a process with a window of its own (`windows`: an app started from an own app without a unit of
/// its own, as without systemd's user manager process_scope.rs runs it as a plain child, is that app with its
/// children, not part of the own app) and at this host (whose children are its own: host_children). A process
/// with a window itself is never a helper. Not caught: what runs between the own app and such a window (sh -c,
/// bwrap) and apps without a window (yet).
fn own_window_of(
    pid: u32,
    procs: &BTreeMap<u32, Proc>,
    statics: &HashMap<(u32, u64), Static>,
    app_of: &HashMap<u32, String>,
    own: &HashSet<u32>,
    windows: &BTreeMap<u32, Vec<Value>>,
    host: u32,
) -> Option<String> {
    if windows.contains_key(&pid) {
        return None;
    }
    let p = procs.get(&pid)?;
    let cgroup = &statics.get(&(pid, p.start))?.cgroup;
    let mut q = p.ppid;
    for _ in 0..32 {
        if q <= 1 || q == host || q == pid {
            return None;
        }
        let parent = procs.get(&q)?;
        if statics
            .get(&(q, parent.start))
            .is_none_or(|s| s.cgroup != *cgroup)
        {
            return None;
        }
        if let Some(key) = app_of.get(&q).filter(|_| own.contains(&q)) {
            return Some(key.clone());
        }
        if app_of.contains_key(&q) || windows.contains_key(&q) {
            return None;
        }
        q = parent.ppid;
    }
    None
}
/// Task Manager's name of an own app in `lang`, and its icon (the second of apps/catalog-data.json). Default apps
/// and the Installer are named as their windows are (the native apps' titles).
pub(super) fn own_names(lang: impl crate::i18n::AsLang, app: &str) -> (String, &'static str) {
    let pick = |nl: &str, en: &str| crate::i18n::pick(&lang, nl, en).to_owned();
    match app {
        "settings" => (pick("Instellingen", "Settings"), "preferences-system"),
        "taskmgr" => (
            pick("Taakbeheer", "Task Manager"),
            "utilities-system-monitor",
        ),
        "explorer" => (pick("Verkenner", "File Explorer"), "system-file-manager"),
        "alles" => (pick("Alles", "Alles"), "system-search"),
        "defaultapps" => (
            crate::i18n::tr(&lang, "sysmon.ownApp.defaultapps"),
            "preferences-desktop-default-applications",
        ),
        "installer" => (
            crate::i18n::tr(&lang, "sysmon.ownApp.installer"),
            "system-software-install",
        ),
        _ => (pick("Schijven", "Disks"), "org.gnome.DiskUtility"),
    }
}
pub(super) fn boot() -> f64 {
    static BOOT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *BOOT.get_or_init(|| {
        read("/proc/stat")
            .lines()
            .find_map(|s| s.strip_prefix("btime ").and_then(num))
            .unwrap_or_else(|| {
                wall()
                    - read("/proc/uptime")
                        .split_whitespace()
                        .next()
                        .and_then(num)
                        .unwrap_or(0.)
            })
    })
}
/// Process trees a test starts itself, to see how Task Manager takes them (only argv[0] is borrowed: the
/// programs are bash and sleep). Nothing outside such a tree is signalled.
#[cfg(test)]
pub(super) mod tree {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    /// `bash -c script` with `argv0` as its argv[0], in a process group of its own (its children name themselves
    /// with `exec -a`). Dropped, the group is killed and the root reaped; every process in it ends within 30 s anyway.
    pub(crate) struct Tree(Child);
    impl Tree {
        pub(crate) fn spawn(argv0: &str, script: &str) -> Tree {
            Tree(
                Command::new("bash")
                    .arg0(argv0)
                    .args(["-c", script])
                    .process_group(0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("bash"),
            )
        }
        pub(crate) fn root(&self) -> u32 {
            self.0.id()
        }
        /// The child of `pid` whose argv[0] is `argv0`, once it has exec'd (3 s at most).
        pub(crate) fn child(&self, pid: u32, argv0: &str) -> u32 {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let found = super::scan_proc().into_iter().find(|(c, p)| {
                    p.ppid == pid && super::argv_of(*c).first().is_some_and(|a| a == argv0)
                });
                if let Some((c, _)) = found {
                    return c;
                }
                assert!(Instant::now() < deadline, "no child {argv0} of {pid}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: kill(2) of this tree's own process group (process_group(0): its id is the root's pid).
            unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
            let _ = self.0.wait();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn desktop_entry(id:&str)->Desktop {
        Desktop{id:id.into(),name:id.trim_end_matches(".desktop").into(),icon:None,hidden:false,terminal:false}
    }
    fn executable_static(path:&str)->Static {
        Static{cmd:String::new(),argv:vec![],exe:Some(path.into()),cgroup:String::new(),unit:None,user:String::new()}
    }
    #[test]
    fn installed_app_paths_and_private_helpers_have_one_identity() {
        for executable in ["/usr/lib/notebird/NoteBird","/opt/Blue-Editor/blue-editor-bin","/opt/sketch.AppDir/sketch.bin"] {
            let mut index=DesktopIndex::default();let app=desktop_entry("example.desktop");
            index.index_path(Path::new(executable),&app);
            assert_eq!(index.process(&executable_static(executable),"app").unwrap().id,app.id);
            let directory=application_directory(executable).unwrap();
            let helper=format!("{directory}/tools/crash-reporter");
            assert_eq!(index.process(&executable_static(&helper),"reporter").unwrap().id,app.id);
        }
    }
    #[test]
    fn shared_binary_directories_do_not_claim_other_apps() {
        for (executable,unrelated) in [
            ("/usr/bin/first","/usr/bin/second"),
            ("/usr/libexec/first","/usr/libexec/second"),
            ("/usr/lib/x86_64-linux-gnu/first","/usr/lib/x86_64-linux-gnu/second"),
            ("/home/example/Applications/First.AppImage","/home/example/Applications/Second.AppImage"),
            ("/opt/shared/first","/opt/shared/second"),
        ] {
            let mut index=DesktopIndex::default();index.index_path(Path::new(executable),&desktop_entry("first.desktop"));
            assert!(index.process(&executable_static(unrelated),"second").is_none(),"{unrelated}");
        }
    }
    #[test]
    fn shared_executables_and_installations_are_ambiguous_not_last_writer_wins() {
        let mut index=DesktopIndex::default();
        for id in ["first.desktop","second.desktop"] {
            let entry=desktop_entry(id);index.index_path(Path::new("/opt/runtime/runtime"),&entry);
            DesktopIndex::put_unique(&mut index.by_exec,&mut index.ambiguous_exec,"runtime".into(),&entry);
        }
        assert!(index.process(&executable_static("/opt/runtime/runtime"),"runtime").is_none());
        assert!(index.process(&executable_static("/opt/runtime/crash-reporter"),"reporter").is_none());
        assert!(index.executable("runtime").is_none());
    }
    #[test]
    fn generic_interpreters_never_become_executable_owners() {
        let mut index=DesktopIndex::default();
        for path in ["/usr/bin/python3","/usr/bin/bash","/usr/bin/node"] {
            index.index_path(Path::new(path),&desktop_entry("some-script.desktop"));
            assert!(index.process(&executable_static(path),basename(path)).is_none(),"{path}");
        }
    }
    #[test]
    fn deleted_executable_path_keeps_its_actual_app_identity() {
        let mut index=DesktopIndex::default();index.index_path(Path::new("/opt/notebird/notebird"),&desktop_entry("notebird.desktop"));
        assert_eq!(index.process(&executable_static("/opt/notebird/notebird (deleted)"),"notebird").unwrap().id,"notebird.desktop");
    }
    #[test]
    fn raw_runtime_app_id_does_not_override_a_different_known_app_executable() {
        let mut index=DesktopIndex::default();
        for (id,path) in [("runtime.desktop","/opt/runtime/runtime"),("notes.desktop","/opt/notes/notes")] {
            let entry=desktop_entry(id);index.by_id.insert(id.trim_end_matches(".desktop").into(),entry.clone());index.index_path(Path::new(path),&entry);
        }
        let window=json!({"desktopId":"","appId":"runtime"});
        assert_eq!(index.window(&executable_static("/opt/notes/notes"),&window).unwrap().id,"notes.desktop");
    }
    #[test]
    fn explicit_pwa_window_survives_shared_executable_and_opaque_wrapper() {
        let mut index=DesktopIndex::default();
        for id in ["runtime.desktop","my-site.desktop"] {
            let entry=desktop_entry(id);index.by_id.insert(id.trim_end_matches(".desktop").into(),entry.clone());index.index_path(Path::new("/opt/runtime/runtime"),&entry);
        }
        let window=json!({"desktopId":"my-site.desktop"});
        assert_eq!(index.window(&executable_static("/opt/runtime/runtime"),&window).unwrap().id,"my-site.desktop");
        let wrapper=desktop_entry("wrapper-app.desktop");index.by_id.insert("wrapper-app".into(),wrapper.clone());index.index_path(Path::new("/opt/wrapper/AppRun"),&wrapper);
        let actual=desktop_entry("actual.desktop");index.index_path(Path::new("/opt/actual/actual"),&actual);
        assert_eq!(index.window(&executable_static("/opt/actual/actual"),&json!({"desktopId":"wrapper-app.desktop"})).unwrap().id,"wrapper-app.desktop");
    }
    #[test]
    fn only_old_external_launcher_units_get_the_external_fallback() {
        for (name,wanted) in [("app-win11-external-123.service",true),("app-win11-panel-123.service",false),("other-external.service",false)] {
            let unit=parse_cgroup(&format!("/user.slice/user-1000.slice/user@1000.service/app.slice/{name}")).unwrap();assert_eq!(external_unit(&unit),wanted);
        }
    }
    fn args(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|s| s.to_string()).collect()
    }

    fn service_fixture(rows: &[(u32, u32, &str)]) -> (BTreeMap<u32, Proc>, HashMap<(u32, u64), Static>) {
        let uid = unsafe { libc::getuid() };
        let mut procs = BTreeMap::new();
        let mut statics = HashMap::new();
        for &(pid, ppid, cgroup) in rows {
            assert!(parse_cgroup(cgroup).is_some(), "invalid service fixture: {cgroup}");
            procs.insert(pid, Proc { pid, ppid, comm: "worker".into(), state: "S".into(), ticks: 0,
                nice: 0, threads: 1, start: u64::from(pid), private: 1024, uid });
            statics.insert((pid, u64::from(pid)), Static { cmd: String::new(), argv: vec![], exe: None,
                cgroup: cgroup.into(), unit: parse_cgroup(cgroup), user: String::new() });
        }
        (procs, statics)
    }

    #[test]
    fn window_runtime_identity_reaches_other_units_without_a_desktop_file() {
        let (procs,mut statics)=service_fixture(&[(100,1,"/app.slice/app-generic-runtime.scope"),
            (101,100,"/app.slice/app-win11-external-1.service"),(102,1,"/app.slice/app-win11-external-1.service")]);
        for (pid,path) in [(100,"/opt/notebird/NoteBird"),(101,"/opt/notebird/NoteBird"),(102,"/opt/notebird/crash-reporter")] {
            statics.get_mut(&(pid,u64::from(pid))).unwrap().exe=Some(path.into());
        }
        let apps=HashMap::from([(100,"win:notebird".into()),(101,"wrong-scope.desktop".into())]);
        let meta=HashMap::from([("win:notebird".into(),json!({"name":"NoteBird"}))]);
        let index=runtime_app_identities(&apps,&HashSet::from([100]),&procs,&statics,&meta);
        for pid in [101,102] {assert_eq!(index.process(&statics[&(pid,u64::from(pid))],"helper").unwrap().id,"win:notebird");}
    }
    #[test]
    fn runtime_identity_rejects_foreign_uids_recycled_pids_and_weak_scopes() {
        let (mut procs,mut statics)=service_fixture(&[(100,1,"/app.slice/app-generic-runtime.scope")]);
        statics.get_mut(&(100,100)).unwrap().exe=Some("/opt/notebird/NoteBird".into());
        let apps=HashMap::from([(100,"note.desktop".into())]);let meta=HashMap::from([("note.desktop".into(),json!({"name":"Note"}))]);
        let target=executable_static("/opt/notebird/NoteBird");
        assert!(runtime_app_identities(&apps,&HashSet::new(),&procs,&statics,&meta).process(&target,"app").is_none());
        procs.get_mut(&100).unwrap().uid+=1;
        assert!(runtime_app_identities(&apps,&HashSet::from([100]),&procs,&statics,&meta).process(&target,"app").is_none());
        procs.get_mut(&100).unwrap().uid-=1;procs.get_mut(&100).unwrap().start+=1;
        assert!(runtime_app_identities(&apps,&HashSet::from([100]),&procs,&statics,&meta).process(&target,"app").is_none());
    }

    fn classify_service(pid: u32, apps: &HashMap<u32, String>, procs: &BTreeMap<u32, Proc>,
        statics: &HashMap<(u32, u64), Static>, critical: &HashSet<u32>) -> Option<String> {
        let services = application_services(apps, procs, statics);
        resolve(pid, 0, apps, &services, &mut HashMap::new(), procs, statics, critical,
            &DesktopIndex::default(), &mut HashMap::new())
    }

    #[test]
    fn every_application_service_keeps_its_helpers_in_one_app_group() {
        let a = "/user.slice/user-1000.slice/user@1000.service/app.slice/one.service";
        let b = "/user.slice/user-1000.slice/user@1000.service/app.slice/another.service";
        let (mut procs, statics) = service_fixture(&[
            (100, 1, a), (101, 100, a), (102, 1, a), // window, child and reparented helper
            (200, 1, b), (201, 200, b),
            (300, 100, "/user.slice/user-1000.slice/user@1000.service/app.slice/unrelated.service"),
            (400, 100, "/other.slice/user@1000.service/app.slice/one.service"), // same unit name, different service instance
            (500, 100, a), (600, 100, a),
        ]);
        procs.get_mut(&500).unwrap().uid += 1;
        let apps = HashMap::from([(100, "browser.desktop".into()), (200, "editor.desktop".into())]);
        let critical = HashSet::from([100, 600]); // known protected app owner still owns its ordinary workers
        for pid in [100, 101, 102] {
            assert_eq!(classify_service(pid, &apps, &procs, &statics, &critical).as_deref(), Some("browser.desktop"));
        }
        for pid in [200, 201] {
            assert_eq!(classify_service(pid, &apps, &procs, &statics, &critical).as_deref(), Some("editor.desktop"));
        }
        for pid in [300, 400, 500, 600] {
            assert_eq!(classify_service(pid, &apps, &procs, &statics, &critical), None, "{pid}");
        }
    }

    #[test]
    fn different_apps_in_one_service_keep_their_own_descendants() {
        let unit = "/user.slice/user-1000.slice/user@1000.service/app.slice/shared-launcher.service";
        let (procs, statics) = service_fixture(&[
            (100, 1, unit), (101, 100, unit), (200, 1, unit), (201, 200, unit), (300, 1, unit),
        ]);
        let apps = HashMap::from([(100, "first.desktop".into()), (200, "second.desktop".into())]);
        let critical = HashSet::new();
        assert_eq!(classify_service(101, &apps, &procs, &statics, &critical).as_deref(), Some("first.desktop"));
        assert_eq!(classify_service(201, &apps, &procs, &statics, &critical).as_deref(), Some("second.desktop"));
        assert_eq!(classify_service(300, &apps, &procs, &statics, &critical), None);
    }

    #[test]
    fn process_response_lists_service_app_and_worker_once_with_their_combined_memory() {
        let app = tree::Tree::spawn("/nonexistent/browser-fixture",
            "(exec -a /nonexistent/browser-worker sleep 30) & wait");
        let worker = app.child(app.root(), "/nonexistent/browser-worker");
        let root = std::env::temp_dir().join(format!("win11-sysmon-service-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("settings.json"), json!({"language":"en"}).to_string()).unwrap();
        let ctx = ServiceContext::new(crate::events::Events::new(&root.join("events")).unwrap(),
            config, root.join("cache"), root.join("ui"), "taskmgr".into(), false).unwrap();
        let mut pm = ProcessState::default();
        pm.units_desc = (now(), HashMap::from([("cached.service".into(), "fixture".into())]));
        pm.windows = (now(), vec![json!({"pid":app.root(),"appId":"browser-fixture",
            "desktopId":"","title":"Browser fixture"})]);
        let procs = scan_proc();
        for pid in [app.root(), worker] {
            let p = &procs[&pid];
            let mut st = static_of(p);
            st.cgroup = "/user.slice/user-1000.slice/user@1000.service/app.slice/arbitrary-browser-launch.service".into();
            st.unit = parse_cgroup(&st.cgroup);
            assert!(st.unit.is_some());
            pm.statics.insert((pid, p.start), st);
        }
        let data = pm.processes(&ctx, true);
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
        let rows = data["processes"].as_array().unwrap();
        let group = data["apps"].as_array().unwrap().iter()
            .find(|g| g["pids"].as_array().unwrap().iter().any(|pid| *pid == app.root())).unwrap();
        let pids = group["pids"].as_array().unwrap();
        for pid in [app.root(), worker] {
            assert!(pids.iter().any(|p| *p == pid), "missing {pid}");
            let occurrences = ["apps", "background", "system"].iter()
                .flat_map(|section| data[*section].as_array().unwrap())
                .filter(|g| g["pids"].as_array().unwrap().iter().any(|p| *p == pid)).count();
            assert_eq!(occurrences, 1, "{pid} must appear in exactly one group");
        }
        let combined: u64 = rows.iter().filter(|p| pids.contains(&p["pid"]))
            .map(|p| p["memory"].as_u64().unwrap()).sum();
        assert_eq!(group["memory"].as_u64(), Some(combined));
    }

    #[test]
    fn split_launcher_apps_and_reparented_helpers_keep_one_real_app_group() {
        let main=tree::Tree::spawn("/nonexistent/app-main","sleep 30 & wait");
        let child=tree::Tree::spawn("/nonexistent/app-worker","sleep 30 & wait");
        let orphan=tree::Tree::spawn("/nonexistent/app-reporter","sleep 30 & wait");
        let root=std::env::temp_dir().join(format!("win11-sysmon-split-{}",crate::events::nonce()));
        let config=root.join("config");fs::create_dir_all(&config).unwrap();
        fs::write(config.join("settings.json"),json!({"language":"en"}).to_string()).unwrap();
        let ctx=ServiceContext::new(crate::events::Events::new(&root.join("events")).unwrap(),
            config,root.join("cache"),root.join("ui"),"taskmgr".into(),false).unwrap();
        let mut state=ProcessState::default();
        state.desktop.built=now();state.desktop.lang="en".into();
        let app=desktop_entry("notebird.desktop");let runtime=desktop_entry("generic-runtime.desktop");
        state.desktop.by_id.insert("notebird".into(),app.clone());
        state.desktop.by_id.insert("generic-runtime".into(),runtime.clone());
        state.desktop.index_path(Path::new("/opt/notebird/NoteBird"),&app);
        state.desktop.index_path(Path::new("/opt/generic-runtime/generic-runtime"),&runtime);
        state.windows=(now(),vec![json!({"pid":main.root(),"desktopId":"","appId":"generic-runtime","title":"Actual application"})]);
        state.units_desc=(now(),HashMap::from([("cached.service".into(),"fixture".into())]));
        let processes=scan_proc();
        for (pid,path,unit) in [
            (main.root(),"/opt/notebird/NoteBird","/app.slice/app-generic-runtime-1.scope"),
            (child.root(),"/opt/notebird/NoteBird","/app.slice/app-win11-external-1.service"),
            (orphan.root(),"/opt/notebird/crash-reporter","/app.slice/app-win11-external-1.service"),
        ] {
            let p=&processes[&pid];let mut st=static_of(p);st.exe=Some(path.into());st.cgroup=unit.into();st.unit=parse_cgroup(unit);
            state.statics.insert((pid,p.start),st);
        }
        let answer=state.processes(&ctx,true);ctx.events.shutdown();let _=fs::remove_dir_all(root);
        let group=answer["apps"].as_array().unwrap().iter().find(|g|g["key"]=="notebird.desktop").unwrap();
        for pid in [main.root(),child.root(),orphan.root()] {
            assert!(group["pids"].as_array().unwrap().iter().any(|value|value.as_u64()==Some(pid.into())),"missing {pid}");
            let occurrences=["apps","background","system"].iter().flat_map(|section|answer[*section].as_array().unwrap())
                .filter(|g|g["pids"].as_array().unwrap().iter().any(|value|value.as_u64()==Some(pid.into()))).count();
            assert_eq!(occurrences,1,"double counted {pid}");
        }
        let sum:u64=answer["processes"].as_array().unwrap().iter()
            .filter(|p|group["pids"].as_array().unwrap().contains(&p["pid"]))
            .map(|p|p["memory"].as_u64().unwrap()).sum();
        assert_eq!(group["memory"].as_u64(),Some(sum));
    }

    #[test]
    fn current_native_app_backend_is_accounted_with_its_own_window() {
        let worker = tree::Tree::spawn("/nonexistent/taskmgr-helper", "sleep 30");
        let host = std::process::id();
        let procs = scan_proc();
        let parent = procs[&host].ppid;
        let root = std::env::temp_dir().join(format!("win11-sysmon-self-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("settings.json"), json!({"language":"en"}).to_string()).unwrap();
        let ctx = ServiceContext::new(crate::events::Events::new(&root.join("events")).unwrap(),
            config, root.join("cache"), root.join("ui"), "taskmgr".into(), false).unwrap();
        let mut pm = ProcessState::default();
        pm.units_desc = (now(), HashMap::from([("cached.service".into(), "fixture".into())]));
        pm.windows = (now(), vec![]);
        for pid in [parent, host, worker.root()] {
            let p = &procs[&pid];
            let mut st = static_of(p);
            st.cgroup = "/user.slice/user@1000.service/app.slice/app-win11-taskmgr-fixture.service".into();
            st.unit = parse_cgroup(&st.cgroup);
            if pid == parent { st.argv = args(&["/nonexistent/win11-gtk-taskmgr"]); }
            if pid == host { st.argv = args(&["/nonexistent/win11-app", "--app", "taskmgr", "--http-only"]); }
            pm.statics.insert((pid, p.start), st);
        }
        let data = pm.processes(&ctx, true);
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
        for pid in [parent, host, worker.root()] {
            let row = data["processes"].as_array().unwrap().iter().find(|p| p["pid"] == pid).unwrap();
            assert_eq!(row["group"], "app", "{pid}");
            assert_eq!(row["groupKey"], "win11shell-taskmgr.desktop", "{pid}");
        }
    }
    #[test]
    fn own_app_knows_the_native_apps_and_their_launchers() {
        for kind in OWN_KINDS {
            let native = format!("/home/u/.local/bin/win11-gtk-{kind}");
            assert_eq!(own_app(&args(&[&native]), &[]).as_deref(), Some(kind));
            // as host lib.rs exec_native_app execs it: the launcher's arguments after `--app <kind>`
            assert_eq!(
                own_app(
                    &args(&[&native, "--params", "{}"]),
                    &args(&["/usr/bin/gnome-shell"])
                )
                .as_deref(),
                Some(kind)
            );
            // the launcher before its exec
            assert_eq!(
                own_app(&args(&["/usr/bin/win11-app", "--app", kind]), &[]).as_deref(),
                Some(kind)
            );
        }
        for (launcher, kind) in [
            ("win11-explorer", "explorer"),
            ("win11-default-apps", "defaultapps"),
            ("win11-installer", "installer"),
        ] {
            assert_eq!(
                own_app(&args(&[&format!("/opt/w/{launcher}")]), &[]).as_deref(),
                Some(kind)
            );
        }
        let mut kinds = OWN_KINDS.to_vec();
        let mut launch = crate::launch::OWN_APPS.to_vec();
        kinds.sort_unstable();
        launch.sort_unstable();
        assert_eq!(kinds, launch);
        let none: [&[&str]; 13] = [
            &[],
            &["/x/win11-gtk-"],
            &["/x/win11-gtk-calendar"],
            &["/x/win11-gtk-panel"],
            &["/x/win11-shell"],
            &["/x/win11-calendar"],
            &["/x/win11-panel"],
            &["/x/win11-file-portal"],
            &["/x/win11-index-worker"],
            &["/x/win11-app", "--app", "panel"],
            &["/x/win11-app", "--app"],
            &["/x/win11-app", "--gnome-look"],
            &["/usr/bin/nautilus", "--app", "explorer"],
        ];
        for argv in none {
            assert_eq!(own_app(&args(argv), &[]), None, "{argv:?}");
        }
    }
    #[test]
    fn a_backend_is_part_of_its_window_not_an_app_of_its_own() {
        for kind in OWN_KINDS {
            let window = args(&[&format!("/x/win11-gtk-{kind}")]);
            assert_eq!(
                own_app(
                    &args(&["/x/win11-app", "--app", kind, "--http-only"]),
                    &window
                ),
                None
            );
            assert_eq!(
                own_app(
                    &args(&["/x/win11-app", "--app", kind, "--http-only", "--dry-run"]),
                    &window
                ),
                None
            );
            assert_eq!(own_app(&args(&[&window[0], "--http-only"]), &[]), None);
        }
        assert_eq!(
            own_app(
                &args(&["/x/win11-panel", "--http-only"]),
                &args(&["/x/win11-shell"])
            ),
            None
        );
    }
    #[test]
    fn the_portal_file_picker_is_not_explorer() {
        let explorer = |extra: &[&str], parent: &[&str]| {
            let mut argv = vec!["/x/win11-gtk-explorer"];
            argv.extend(extra);
            own_app(&args(&argv), &args(parent))
        };
        let portal = ["/home/u/.local/bin/win11-file-portal"];
        assert_eq!(explorer(&["--picker-stdio"], &portal), None);
        assert_eq!(explorer(&["--picker-stdio"], &[]), None);
        assert_eq!(explorer(&[], &portal), None);
        assert_eq!(
            explorer(&["--params", r#"{"picker":{"mode":"open"}}"#], &[]),
            None
        );
        assert_eq!(
            explorer(&["--params", r#"{"picker":"{\"mode\":\"save\"}"}"#], &[]),
            None
        );
        assert_eq!(
            own_app(&args(&["/x/win11-explorer", "--picker-stdio"]), &[]),
            None
        );
        // a regular Explorer: other params, broken params, another parent
        let regular: [(&[&str], &[&str]); 6] = [
            (&[], &["/usr/bin/gnome-shell"]),
            (&["--params", r#"{"path":"/tmp","picker":null}"#], &[]),
            (&["--params", r#"{"picker":"open"}"#], &[]),
            (&["--params", "{"], &[]),
            (&["/tmp", "--params"], &[]),
            (&["/tmp"], &["/x/win11-file-portal-old"]),
        ];
        for (extra, parent) in regular {
            assert_eq!(
                explorer(extra, parent).as_deref(),
                Some("explorer"),
                "{extra:?} {parent:?}"
            );
        }
        // only Explorer can be the picker
        assert_eq!(
            own_app(
                &args(&["/x/win11-gtk-settings", "--picker-stdio"]),
                &args(&portal)
            )
            .as_deref(),
            Some("settings")
        );
    }
    #[test]
    fn own_names_cover_every_kind_in_both_languages() {
        let (mut nls, mut ens) = (HashSet::new(), HashSet::new());
        for kind in OWN_KINDS {
            let (nl, icon) = own_names("nl", kind);
            let (en, en_icon) = own_names(crate::i18n::Lang::En, kind);
            assert!(
                !nl.is_empty() && !en.is_empty() && !icon.is_empty(),
                "{kind}"
            );
            assert_eq!(icon, en_icon);
            assert!(
                nls.insert(nl) && ens.insert(en),
                "{kind}: a name of its own"
            );
        }
        assert_eq!(
            own_names("nl", "taskmgr"),
            ("Taakbeheer".into(), "utilities-system-monitor")
        );
        assert_eq!(
            own_names("en", "disks"),
            ("Disks".into(), "org.gnome.DiskUtility")
        );
        assert_eq!(
            own_names("nl", "defaultapps"),
            (
                "Standaard-apps".into(),
                "preferences-desktop-default-applications"
            )
        );
        assert_eq!(own_names("en", "defaultapps").0, "Default apps");
        assert_eq!(
            own_names("nl", "installer"),
            ("App-installatieprogramma".into(), "system-software-install")
        );
        assert_eq!(own_names("en", "installer").0, "App Installer");
    }
    #[test]
    fn processes_below_an_own_window_in_its_unit_belong_to_it() {
        let unit = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-win11-explorer-7-1-0.service";
        let other = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-org.gnome.TextEditor-9.scope";
        // 10 this host; 100 Explorer's window, 101 its backend, 102 a worker of the backend; 103 an app the window
        // started in a unit of its own, 104 that app's child; 105 another app's window in Explorer's unit (no
        // systemd: one cgroup for all), 106 its child; 107 a child of the host in the same cgroup
        let rows: [(u32, u32, &[&str], &str); 9] = [
            (100, 1, &["/x/win11-gtk-explorer"], unit),
            (
                101,
                100,
                &["/x/win11-app", "--app", "explorer", "--http-only"],
                unit,
            ),
            (102, 101, &["/x/win11-index-worker"], unit),
            (103, 100, &["/usr/bin/gnome-text-editor"], other),
            (104, 103, &["/usr/bin/aspell"], other),
            (105, 100, &["/usr/bin/gedit"], unit),
            (106, 105, &["/usr/bin/aspell"], unit),
            (107, 10, &["/x/win11-image-worker"], unit),
            (
                10,
                100,
                &["/x/win11-app", "--app", "taskmgr", "--http-only"],
                unit,
            ),
        ];
        let procs = rows
            .iter()
            .map(|(pid, ppid, _, _)| {
                let p = Proc {
                    pid: *pid,
                    ppid: *ppid,
                    comm: String::new(),
                    state: "S".into(),
                    ticks: 0,
                    nice: 0,
                    threads: 1,
                    start: u64::from(*pid),
                    private: 0,
                    uid: 1000,
                };
                (*pid, p)
            })
            .collect::<BTreeMap<_, _>>();
        let statics = rows
            .iter()
            .map(|(pid, _, argv, cgroup)| {
                let st = Static {
                    cmd: argv.join(" "),
                    argv: args(argv),
                    exe: None,
                    cgroup: (*cgroup).into(),
                    unit: parse_cgroup(cgroup),
                    user: String::new(),
                };
                ((*pid, u64::from(*pid)), st)
            })
            .collect::<HashMap<_, _>>();
        // as processes() calls it: only the own windows have an app yet; the windows cache knows gedit's window
        let explorer = "win11shell-explorer.desktop".to_string();
        let app_of = HashMap::from([(100, explorer.clone())]);
        let own = HashSet::from([100]);
        let windows = BTreeMap::from([
            (100, vec![json!({"pid":100})]),
            (105, vec![json!({"pid":105})]),
        ]);
        let of = |pid| own_window_of(pid, &procs, &statics, &app_of, &own, &windows, 10);
        assert_eq!(of(101), Some(explorer.clone()));
        assert_eq!(of(102), Some(explorer));
        for pid in [103, 104, 105, 106, 107, 999] {
            assert_eq!(of(pid), None, "{pid}");
        }
    }
    // The native app as it runs (host lib.rs exec_native_app, native/kit backend.rs): its window `win11-gtk-disks`
    // with the backend `win11-app --app disks --http-only` below it, and that backend's own child, is one app row;
    // an Explorer started by win11-file-portal (the file picker) is no Explorer row.
    #[test]
    fn task_manager_shows_a_native_app_with_its_backend_and_not_the_portal_picker() {
        let dir = "/nonexistent/winpoint-test";
        let app = tree::Tree::spawn(
            &format!("{dir}/win11-gtk-disks"),
            &format!(
                "(exec -a {dir}/win11-app bash -c 'sleep 30; :' x --app disks --http-only) & wait"
            ),
        );
        let backend = app.child(app.root(), &format!("{dir}/win11-app"));
        let worker = app.child(backend, "sleep");
        let portal = tree::Tree::spawn(
            &format!("{dir}/win11-file-portal"),
            &format!("(exec -a {dir}/win11-gtk-explorer sleep 30) & wait"),
        );
        let picker = portal.child(portal.root(), &format!("{dir}/win11-gtk-explorer"));
        let root =
            std::env::temp_dir().join(format!("win11-sysmon-own-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            json!({"language":"en"}).to_string(),
        )
        .unwrap();
        let ctx = ServiceContext::new(
            crate::events::Events::new(&root.join("events")).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "taskmgr".into(),
            false,
        )
        .unwrap();
        let mut pm = ProcessState::default();
        // (cached unit descriptions keep systemctl out of the test)
        pm.units_desc = (now(), HashMap::from([("x.service".into(), "x".into())]));
        let data = pm.processes(&ctx, true);
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
        let rows = data["processes"].as_array().unwrap();
        let row = |pid: u32| rows.iter().find(|p| p["pid"] == pid).unwrap().clone();
        for pid in [app.root(), backend, worker] {
            assert_eq!(row(pid)["groupKey"], "win11shell-disks.desktop", "{pid}");
            assert_eq!(row(pid)["group"], "app", "{pid}");
        }
        let disks = data["apps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["key"] == "win11shell-disks.desktop")
            .unwrap();
        assert_eq!(disks["name"], "Disks");
        assert_eq!(disks["icon"], "org.gnome.DiskUtility");
        for pid in [app.root(), backend, worker] {
            assert!(
                disks["pids"].as_array().unwrap().contains(&json!(pid)),
                "{pid}"
            );
        }
        // the picker (here a child of this host, through the portal) is never counted as Explorer. Which other group
        // it lands in depends on where the test runs: started from an app's scope (a terminal or an editor), every
        // process of that scope is that app's, the picker too.
        assert_ne!(row(picker)["groupKey"], "win11shell-explorer.desktop");
        assert!(
            data["apps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|g| g["key"] == "win11shell-explorer.desktop")
                .all(|g| !g["pids"].as_array().unwrap().contains(&json!(picker)))
        );
    }
    #[test]
    fn recycled_pid_does_not_inherit_cpu_accounting() {
        let mut state = ProcessState::default();
        state.record(1., BTreeMap::from([(3, (100, 100))]));
        let next = state.record(2., BTreeMap::from([(3, (200, 1000))]));
        assert!(next.is_empty());
    }
    #[test]
    fn cpu_delta_is_machine_share() {
        let mut s = ProcessState::default();
        s.record(1., BTreeMap::from([(3, (100, 100))]));
        let c = s.record(2., BTreeMap::from([(3, (100, 200))]));
        assert_eq!(c[&(3, 100)], 100. * 100. / (clk() * ncpu() as f64));
    }
    #[test]
    fn stat_reader_handles_comm_parentheses_and_existing_self() {
        let (comm, f) = stat_of(std::process::id()).unwrap();
        assert!(!comm.is_empty());
        assert!(f.len() > 19);
    }
    // Review m2/n3: the health page's busy-process names (health::busy) follow a language switch. The process list
    // is built again only when its names are in the other language; with nothing busy, or before the first list
    // (names from /proc, as in 3b898e6), nothing extra runs.
    #[test]
    fn busy_names_are_rebuilt_only_after_a_language_switch() {
        let root =
            std::env::temp_dir().join(format!("win11-sysmon-busy-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            json!({"language":"en"}).to_string(),
        )
        .unwrap();
        let ctx = ServiceContext::new(
            crate::events::Events::new(&root.join("events")).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "taskmgr".into(),
            false,
        )
        .unwrap();
        let pid = std::process::id();
        let start = stat_of(pid).unwrap().1[19].parse::<u64>().unwrap();
        // A fresh state per case: without CPU samples the list's own record() keeps the seeded busy entry, and
        // cached unit descriptions keep systemctl out of the test (the sandbox's guard stub would log it).
        let state = |busy: bool, names_lang: &str| {
            let mut pm = ProcessState::default();
            pm.units_desc = (now(), HashMap::from([("x.service".into(), "x".into())]));
            if busy {
                pm.high_since.insert((pid, start), now() - 60.);
            }
            if !names_lang.is_empty() {
                pm.last_names
                    .insert((pid, start), ("old-name".into(), "old-group".into()));
            }
            pm.names_lang = names_lang.into();
            pm
        };
        let busy = super::super::health::busy;
        // names in the current language: used as they are, no new list
        let mut pm = state(true, "en");
        let hot = busy(&ctx, &mut pm, "en", 30.);
        assert_eq!((hot.len(), hot[0]["name"].as_str()), (1, Some("old-name")));
        // names in the other language: the list is built again, in the current language
        let mut pm = state(true, "nl");
        let hot = busy(&ctx, &mut pm, "en", 30.);
        assert_eq!((hot.len(), hot[0]["pid"].as_u64()), (1, Some(pid as u64)));
        // (the test binary is this process, which processes() names as the host)
        assert_eq!(hot[0]["name"], tr("en", "host", &[]));
        assert_eq!(hot[0]["groupKey"], "win11shell");
        assert_eq!(pm.names_lang(), "en");
        // nothing busy: no list
        let mut pm = state(false, "nl");
        assert!(busy(&ctx, &mut pm, "en", 30.).is_empty());
        assert_eq!(pm.names_lang(), "nl");
        // no list yet: the name from /proc, as in 3b898e6, and still no list
        let mut pm = state(true, "");
        let hot = busy(&ctx, &mut pm, "en", 30.);
        assert_eq!(hot[0]["name"], read(format!("/proc/{pid}/comm")));
        assert_eq!(pm.names_lang(), "");
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
