//! Last-closed native window state. No GTK object crosses into the worker thread.
use serde_json::{Map, Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};
const SESSION_MAX: usize = 1 << 20;
const FILE_MAX: u64 = 8 << 20;
fn eligible(kind: &str) -> bool {
    matches!(
        kind,
        "explorer" | "settings" | "taskmgr" | "alles" | "disks" | "defaultapps" | "installer"
    )
}
fn rect(value: &Value) -> Option<Value> {
    let mut out = Map::new();
    for key in ["x", "y", "w", "h"] {
        let n = value[key].as_i64()?;
        if !(-1_000_000..=1_000_000).contains(&n)
            || (["w", "h"].contains(&key) && !(100..=20000).contains(&n))
        {
            return None;
        }
        out.insert(key.into(), json!(n));
    }
    Some(Value::Object(out))
}
fn text(v: &Value, max: usize) -> Option<&str> {
    v.as_str()
        .filter(|s| s.chars().count() <= max && !s.contains('\0'))
}
fn location(v: &Value) -> Option<Value> {
    let kind = v["kind"].as_str()?;
    let mut out = json!({"kind":kind});
    match kind {
        "folder" | "search" => {
            let p = text(&v["path"], 16384)?.strip_prefix('/')?;
            out["path"] = json!(format!("/{p}"));
            if kind == "search" {
                out["query"] = json!(text(&v["query"], 4096)?)
            }
        }
        "home" | "thispc" | "gallery" | "trash" => {}
        _ => return None,
    }
    Some(out)
}
fn paths(v: &Value) -> Option<Value> {
    let list = v.as_array()?;
    if list.len() > 10000 {
        return None;
    }
    Some(Value::Array(
        list.iter()
            .map(|v| {
                text(v, 16384)
                    .filter(|s| s.starts_with('/'))
                    .map(|s| json!(s))
            })
            .collect::<Option<Vec<_>>>()?,
    ))
}
fn tab(v: &Value) -> Option<Value> {
    if v["version"] != 1 {
        return None;
    }
    let loc = location(&v["loc"])?;
    let history = v["history"].as_array()?;
    if history.is_empty() || history.len() > 60 {
        return None;
    }
    let history = history.iter().map(location).collect::<Option<Vec<_>>>()?;
    let index = v["hIndex"].as_u64()?;
    if index >= history.len() as u64 {
        return None;
    }
    let filter = text(&v["filter"], 4096)?;
    let memo = v["scrollMemo"].as_array()?;
    if memo.len() > 120 {
        return None;
    }
    let memo = memo
        .iter()
        .map(|entry| {
            let e = entry.as_array()?;
            if e.len() != 2 {
                return None;
            }
            let key = text(&e[0], 20000)?;
            let top = e[1]["top"].as_f64().filter(|n| n.is_finite() && *n >= 0.)?;
            let left = e[1]["left"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)?;
            Some(json!([key,{"top":top,"left":left}]))
        })
        .collect::<Option<Vec<_>>>()?;
    let empty = json!([]);
    let selection = paths(v.get("selection").unwrap_or(&empty))?;
    let focus = if v["focus"].is_null() {
        Value::Null
    } else {
        let p = text(&v["focus"], 16384)?.starts_with('/');
        if !p {
            return None;
        }
        v["focus"].clone()
    };
    let pending = if v["pendingSelect"].is_null() {
        Value::Null
    } else if v["pendingSelect"].is_array() {
        paths(&v["pendingSelect"])?
    } else {
        paths(&json!([v["pendingSelect"]]))?
    };
    let reveal = if v["pendingReveal"].is_null() {
        Value::Null
    } else {
        let s = text(&v["pendingReveal"], 16384)?;
        if !s.starts_with('/') {
            return None;
        }
        json!(s)
    };
    Some(
        json!({"version":1,"loc":loc,"history":history,"hIndex":index,"filter":filter,"scrollMemo":memo,"selection":selection,"focus":focus,"pendingSelect":pending,"pendingProps":v["pendingProps"]==true,"pendingReveal":reveal}),
    )
}
pub(crate) fn session(value: &Value) -> Option<Value> {
    let tabs = value["tabs"].as_array()?;
    if value["version"] != 1
        || tabs.is_empty()
        || tabs.len() > 20
        || serde_json::to_vec(value).ok()?.len() > SESSION_MAX
    {
        return None;
    }
    let tabs = tabs.iter().map(tab).collect::<Option<Vec<_>>>()?;
    let active = value["active"].as_u64()?;
    if active >= tabs.len() as u64 {
        return None;
    }
    Some(json!({"version":1,"tabs":tabs,"active":active}))
}
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn private_file(path: &Path, create: bool) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let st = f.metadata()?;
    if !st.is_file()
        || st.uid() != unsafe { libc::geteuid() }
        || st.nlink() != 1
        || st.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Window state must be private",
        ));
    }
    Ok(f)
}
fn locked(config: &Path) -> io::Result<Lock> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(config)?;
    let st = fs::symlink_metadata(config)?;
    if !st.is_dir() || st.file_type().is_symlink() || st.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid window-state directory",
        ));
    }
    let f = private_file(&config.join(".window-state.lock"), true)?;
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(Lock(f));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}
fn read(config: &Path) -> io::Result<Value> {
    let f = match private_file(&config.join("window-state.json"), false) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(json!({"version":1,"apps":{}})),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    f.take(FILE_MAX + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Window-state file is too large",
        ));
    }
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if v["version"] == 1 && v["apps"].is_object() {
        Ok(v)
    } else {
        Ok(json!({"version":1,"apps":{}}))
    }
}
fn write(config: &Path, kind: &str, value: &Value) -> io::Result<()> {
    let _lock = locked(config)?;
    let mut root = read(config)?;
    root["apps"][kind] = value.clone();
    let bytes = serde_json::to_vec(&root)?;
    if bytes.len() as u64 > FILE_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Window-state file is too large",
        ));
    }
    static ID: AtomicU64 = AtomicU64::new(0);
    let temp = config.join(format!(
        ".window-state.{}.{}.tmp",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        f.write_all(&bytes)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.sync_all()?;
        fs::rename(&temp, config.join("window-state.json"))?;
        if let Ok(dir) = File::open(config) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[derive(Default)]
struct Current {
    normal: Option<Value>,
    maximized: bool,
    session: Option<Value>,
    observed: bool,
}
enum Command {
    Restore,
    Capture,
    Flush(mpsc::SyncSender<()>),
}
pub(crate) struct WindowState {
    loaded: Value,
    current: Arc<Mutex<Current>>,
    sender: mpsc::SyncSender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
    capturing: Arc<AtomicBool>,
    stopped: AtomicBool,
}
impl WindowState {
    pub(crate) fn open(config: &Path, kind: &str, app_id: &str) -> Option<Arc<Self>> {
        if !eligible(kind) {
            return None;
        }
        let loaded = {
            let _lock = locked(config).ok()?;
            read(config).ok()?["apps"][kind].clone()
        };
        let current = Arc::new(Mutex::new(Current {
            normal: rect(&loaded["normalRect"]),
            maximized: loaded["maximized"] == true,
            session: if kind == "explorer" {
                session(&loaded["session"])
            } else {
                None
            },
            observed: false,
        }));
        let (sender, receiver) = mpsc::sync_channel(8);
        let shared = current.clone();
        let capturing = Arc::new(AtomicBool::new(false));
        let busy = capturing.clone();
        let (config, kind, app_id) = (config.to_path_buf(), kind.to_string(), app_id.to_string());
        let saved = loaded.clone();
        let thread = std::thread::Builder::new().name("win11-window-state".into()).spawn(move || {
            let identity = json!({"pid":std::process::id(),"appId":app_id});
            let mut restore = false;
            let mut attempts = 0;
            loop {
                match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(Command::Restore) => restore = true,
                    Ok(Command::Capture) => busy.store(false,Ordering::Release),
                    Ok(Command::Flush(ack)) => {
                        capture(&shared,&identity);
                        let current = shared.lock().unwrap_or_else(|e|e.into_inner());
                        if current.observed || current.session.is_some() {
                            let value = json!({"normalRect":current.normal,"maximized":current.maximized,"session":current.session});
                            let _ = write(&config,&kind,&value);
                        }
                        drop(current);
                        let _ = ack.send(());
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                }
                if restore {
                    if let Some(normal) = rect(&saved["normalRect"]) {
                        let mut args=identity.clone();
                        args["rect"]=normal; args["maximized"]=json!(saved["maximized"]==true);
                        let result = crate::gnome::call("windows.hostRestore",&args);
                        if std::env::var_os("WIN11_DEV_REPORT").is_some() { eprintln!("Win11 window restore {args}: {result:?}"); }
                        if result.is_ok() { restore=false; }
                        else { attempts+=1; if attempts>=12 { restore=false; } }
                    } else { restore=false; }
                }
                capture(&shared,&identity);
            }
        }).ok()?;
        Some(Arc::new(Self {
            loaded,
            current,
            sender,
            thread: Mutex::new(Some(thread)),
            capturing,
            stopped: AtomicBool::new(false),
        }))
    }
    pub(crate) fn normal_rect(&self) -> Option<Value> {
        rect(&self.loaded["normalRect"])
    }
    pub(crate) fn maximized(&self) -> bool {
        self.loaded["maximized"] == true
    }
    pub(crate) fn explorer_session(&self) -> Value {
        session(&self.loaded["session"]).unwrap_or(Value::Null)
    }
    pub(crate) fn restore(&self) {
        let _ = self.sender.try_send(Command::Restore);
    }
    pub(crate) fn capture(&self) {
        if !self.capturing.swap(true, Ordering::AcqRel)
            && self.sender.try_send(Command::Capture).is_err()
        {
            self.capturing.store(false, Ordering::Release);
        }
    }
    pub(crate) fn save_session(&self, value: &Value) -> bool {
        let Some(value) = session(value) else {
            return false;
        };
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session = Some(value);
        true
    }
    pub(crate) fn flush(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        if self.sender.send(Command::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(8));
        }
        if let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = thread.join();
        }
    }
}
impl Drop for WindowState {
    fn drop(&mut self) {
        self.flush();
    }
}
fn capture(current: &Mutex<Current>, identity: &Value) {
    let Ok(reply) = crate::gnome::call("windows.hostState", identity) else {
        return;
    };
    let mut c = current.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(normal) = rect(&reply["normalRect"]).or_else(|| {
        if reply["maximized"] != true {
            rect(&reply["rect"])
        } else {
            None
        }
    }) {
        c.normal = Some(normal)
    }
    if reply["maximized"].is_boolean() {
        c.maximized = reply["maximized"] == true;
        c.observed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "win11-window-state-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn sample() -> Value {
        json!({"version":1,"active":0,"tabs":[{"version":1,"loc":{"kind":"folder","path":"/fixture/A"},"history":[{"kind":"home"},{"kind":"folder","path":"/fixture/A"}],"hIndex":1,"selection":["/fixture/A/a.txt"],"focus":"/fixture/A/a.txt","filter":"a","scrollMemo":[["folder:/fixture/A",{"top":40,"left":0}]],"pendingSelect":null,"pendingProps":false,"pendingReveal":null}]})
    }
    #[test]
    fn persistence_excludes_surfaces_and_pickers() {
        for k in ["start", "desktop", "panel", "picker", "unknown"] {
            assert!(!eligible(k))
        }
        for k in [
            "explorer",
            "settings",
            "taskmgr",
            "alles",
            "disks",
            "defaultapps",
        ] {
            assert!(eligible(k))
        }
    }
    #[test]
    fn semantic_tabs_validate_and_drop_listing_data() {
        let mut v = sample();
        v["tabs"][0]["items"] = json!([{"name":"not persisted"}]);
        let clean = session(&v).unwrap();
        assert!(clean["tabs"][0].get("items").is_none());
        assert_eq!(clean["tabs"][0]["history"], v["tabs"][0]["history"]);
        assert_eq!(clean["tabs"][0]["selection"], v["tabs"][0]["selection"]);
        assert_eq!(clean["active"], 0);
    }
    #[test]
    fn invalid_and_oversized_sessions_are_rejected() {
        for mutate in 0..8 {
            let mut v = sample();
            match mutate {
                0 => v["active"] = json!(1),
                1 => v["tabs"] = json!([]),
                2 => v["tabs"] = json!(vec![sample()["tabs"][0].clone(); 21]),
                3 => v["tabs"][0]["loc"]["path"] = json!("relative"),
                4 => v["tabs"][0]["history"] = json!([]),
                5 => v["tabs"][0]["selection"] = json!(["/bad\0path"]),
                6 => v["tabs"][0]["scrollMemo"][0][1]["top"] = json!(-1),
                _ => v["irrelevant"] = json!("x".repeat(SESSION_MAX)),
            }
            assert!(session(&v).is_none(), "mutation {mutate}");
        }
    }
    #[test]
    fn rectangles_reject_unbounded_or_incomplete_values() {
        assert!(rect(&json!({"x":-1440,"y":0,"w":1200,"h":700})).is_some());
        for v in [
            json!({"x":0,"y":0,"w":0,"h":700}),
            json!({"x":1e12,"y":0,"w":1200,"h":700}),
            json!({"x":0,"y":0,"w":1200}),
            json!({"x":0,"y":0,"w":1200,"h":21000}),
        ] {
            assert!(rect(&v).is_none());
        }
    }
    #[test]
    fn atomic_merge_preserves_other_app_states_and_private_modes() {
        let t = Temp::new();
        let p = Arc::new(t.0.clone());
        let threads = (0..16)
            .map(|n| {
                let p = p.clone();
                std::thread::spawn(move || {
                    write(
                        &p,
                        &format!("app{n}"),
                        &json!({"normalRect":{"x":n,"y":0,"w":800,"h":600},"session":sample()}),
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for t in threads {
            t.join().unwrap()
        }
        let v = read(&p).unwrap();
        assert_eq!(v["apps"].as_object().unwrap().len(), 16);
        for name in ["window-state.json", ".window-state.lock"] {
            assert_eq!(p.join(name).metadata().unwrap().mode() & 0o777, 0o600);
        }
        assert!(
            fs::read_dir(&*p).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }
    #[test]
    fn symlinked_state_and_lock_never_redirect_writes() {
        use std::os::unix::fs::symlink;
        for name in ["window-state.json", ".window-state.lock"] {
            let t = Temp::new();
            let sentinel = t.0.join("sentinel");
            fs::write(&sentinel, b"preserved").unwrap();
            fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o600)).unwrap();
            symlink(&sentinel, t.0.join(name)).unwrap();
            assert!(write(&t.0, "explorer", &sample()).is_err());
            assert_eq!(fs::read(&sentinel).unwrap(), b"preserved");
        }
    }
    #[test]
    fn corrupt_state_recovers_but_oversized_file_is_preserved() {
        let t = Temp::new();
        let p = t.0.join("window-state.json");
        fs::write(&p, b"invalid json").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read(&t.0).unwrap(), json!({"version":1,"apps":{}}));
        let f = OpenOptions::new().write(true).open(&p).unwrap();
        f.set_len(FILE_MAX + 1).unwrap();
        assert!(read(&t.0).is_err());
        assert!(write(&t.0, "explorer", &sample()).is_err());
        assert_eq!(p.metadata().unwrap().len(), FILE_MAX + 1);
    }
}
