//! Separate-process index engine: bounded mapped base, incremental directory
//! updates, original worker wire protocol and per-channel cancellation.
use super::{
    cache::{self, DEL, DIR, Data, HIDDEN, LINK, MOUNT, NOSHOW, Row},
    query::{self, Compiled, Error},
};
use crate::files::ApiError;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::CString,
    fs,
    io::{self, BufRead, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
type Emit = Arc<dyn Fn(Value) + Send + Sync>;
struct State {
    roots: Vec<String>,
    excluded: Vec<String>,
    live: bool,
    configured: bool,
    ready: bool,
    building: bool,
    current: String,
    done: usize,
    total: usize,
    /// Per root: why it is not indexed. Kept as the error (code, key + params): the status carries the key, and the
    /// host renders the text in its language when it relays or returns the status (round2-gate §2).
    errors: HashMap<String, ApiError>,
    refreshing: bool,
    watching: usize,
    last_changed: Instant,
}
impl Default for State {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            excluded: Vec::new(),
            live: true,
            configured: false,
            ready: false,
            building: false,
            current: String::new(),
            done: 0,
            total: 0,
            errors: HashMap::new(),
            refreshing: false,
            watching: 0,
            last_changed: Instant::now(),
        }
    }
}
enum Task {
    Configure(Value),
    Rebuild,
    Directories(Vec<String>),
    Refresh,
    Persist,
    Stop,
}
struct Engine {
    cache: PathBuf,
    data: RwLock<Option<Data>>,
    state: Mutex<State>,
    stopped: AtomicBool,
    cancel: AtomicBool,
    generation: AtomicU64,
    tx: mpsc::Sender<Task>,
    emit: Emit,
    watcher: Mutex<Option<Watcher>>,
}
impl Engine {
    fn new(cache: PathBuf, emit: Emit) -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        let loaded = Data::load(&cache).ok();
        let mut state = State::default();
        state.ready = loaded.as_ref().is_some_and(|d| !d.segments.is_empty());
        let engine = Arc::new(Self {
            cache,
            data: RwLock::new(loaded),
            state: Mutex::new(state),
            stopped: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            tx,
            emit,
            watcher: Mutex::new(None),
        });
        let weak = Arc::downgrade(&engine);
        thread::spawn(move || {
            while let Ok(task) = rx.recv() {
                let Some(e) = weak.upgrade() else { break };
                if matches!(task, Task::Stop) {
                    let _ = e.persist();
                    break;
                }
                match task {
                    Task::Configure(args) => e.configure(args),
                    Task::Rebuild => e.rebuild(),
                    Task::Directories(paths) => e.directories(paths),
                    Task::Refresh => e.refresh(),
                    Task::Persist => {
                        let _ = e.persist();
                    }
                    Task::Stop => {}
                }
            }
        });
        let weak = Arc::downgrade(&engine);
        thread::spawn(move || {
            let mut tick = 0;
            loop {
                thread::sleep(Duration::from_millis(500));
                let Some(e) = weak.upgrade() else { break };
                if e.stopped.load(Ordering::Relaxed) {
                    break;
                }
                e.status_event();
                tick += 1;
                if tick % 1200 == 0 {
                    let _ = e.tx.send(Task::Refresh);
                }
                if tick % 1800 == 0 {
                    let _ = e.tx.send(Task::Persist);
                }
            }
        });
        engine
    }
    fn configure(&self, args: Value) {
        let mounts = cache::mounts();
        let roots = if args["roots"] == "auto"
            || args["roots"] == json!(["auto"])
            || !args["roots"].is_array()
        {
            cache::auto_roots(&mounts)
        } else {
            args["roots"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(cache::normalize)
                .collect()
        };
        let mut errors = HashMap::new();
        let mut clean = Vec::new();
        for p in roots {
            let p = fs::canonicalize(&p)
                .unwrap_or_else(|_| p.into())
                .to_string_lossy()
                .into_owned();
            match fs::metadata(&p) {
                Err(_) => {
                    errors.insert(
                        p,
                        ApiError::tr(
                            "not_found",
                            crate::i18n::current_language(),
                            "index.rootNotFound",
                            crate::i18n::NO_PARAMS,
                        ),
                    );
                    continue;
                }
                Ok(m) if !m.is_dir() => {
                    errors.insert(
                        p,
                        ApiError::tr(
                            "not_a_folder",
                            crate::i18n::current_language(),
                            "index.rootNotFolder",
                            crate::i18n::NO_PARAMS,
                        ),
                    );
                    continue;
                }
                _ => {}
            }
            if !clean.contains(&p) {
                clean.push(p)
            }
        }
        let all = clean.clone();
        clean.retain(|p| {
            !all.iter().any(|other| {
                other != p
                    && cache::beneath(p, other)
                    && fs::metadata(p)
                        .ok()
                        .zip(fs::metadata(other).ok())
                        .is_some_and(|(a, b)| a.dev() == b.dev())
            })
        });
        let excluded = args["excluded"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(cache::normalize)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let unchanged = {
            let data = self.data.read().unwrap_or_else(|e| e.into_inner());
            data.as_ref().is_some_and(|d| {
                d.segments
                    .iter()
                    .map(|s| s.path.clone())
                    .collect::<Vec<_>>()
                    == clean
                    && d.excluded() == excluded
            })
        };
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.roots = clean;
            s.excluded = excluded;
            s.live = args.get("liveUpdate").map(query::truth).unwrap_or(s.live);
            s.errors = errors;
            s.configured = true;
        }
        if unchanged {
            self.setup_watches();
            let _ = self.tx.send(Task::Refresh);
        } else {
            self.rebuild()
        }
        self.status_event();
    }
    fn rebuild(&self) {
        let (roots, excluded) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.building = true;
            s.done = 0;
            s.total = 1;
            s.current = String::new();
            (s.roots.clone(), s.excluded.clone())
        };
        self.cancel.store(false, Ordering::Relaxed);
        let progress = |done: usize, current: &str| {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.done = done;
            s.total = s.total.max(done + 1);
            s.current = current.into();
        };
        let got = cache::build(&self.cache, &roots, &excluded, &self.cancel, &|n, p| {
            progress(n, p);
        });
        match got {
            Ok(mut d) => {
                let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
                d.generation = generation;
                self.state.lock().unwrap_or_else(|e| e.into_inner()).ready = !d.segments.is_empty();
                *self.data.write().unwrap_or_else(|e| e.into_inner()) = Some(d);
                (self.emit)(json!({"event":"changed","data":{"generation":generation}}));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                for root in roots {
                    s.errors.insert(root, cache::root_error(&e));
                }
            }
        }
        self.setup_watches();
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.building = false;
            s.current.clear();
        }
        self.status_event();
    }
    fn state_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.building {
            return "building".into();
        }
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        if d.as_ref().is_some_and(|d| !d.segments.is_empty()) {
            return "ready".into();
        }
        if s.configured && !s.errors.is_empty() {
            "error"
        } else {
            "idle"
        }
        .into()
    }
    fn status(&self) -> Value {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        let (mut files, mut folders) = (0, 0);
        let mut roots = Vec::new();
        let mut built = None;
        let mut overlay = 0;
        if let Some(d) = data.as_ref() {
            let summary = d.summaries();
            files = summary.0;
            folders = summary.1;
            built = d.built;
            overlay = d
                .overlay
                .iter()
                .enumerate()
                .filter(|(i, _)| d.fields(d.n + i).2 & DEL == 0)
                .count();
            for (si, seg) in d.segments.iter().enumerate() {
                let (f, dirs) = summary.2[si];
                roots.push(json!({"path":seg.path,"files":f,"folders":dirs,"fsType":seg.fs,"state":if s.building{"building"}else{"ready"},"builtAt":if seg.built==0.{Value::Null}else{json!(seg.built)}}));
            }
        }
        for root in &s.roots {
            if !roots.iter().any(|r| r["path"] == *root) && s.building {
                roots.push(json!({"path":root,"files":s.done,"folders":0,"fsType":"","state":"building","error":null}));
            }
        }
        for (root, error) in &s.errors {
            if !roots.iter().any(|r| r["path"] == *root) {
                let mut r = json!({"path":root,"files":0,"folders":0,"state":"error","error":error.message});
                if let Some(key) = &error.key {
                    r["errorKey"] = json!(key);
                    r["errorParams"] = error.params.clone().unwrap_or_else(|| json!({}));
                }
                roots.push(r);
            }
        }
        let state = if s.building {
            "building"
        } else if data.as_ref().is_some_and(|d| !d.segments.is_empty()) {
            "ready"
        } else if s.configured && !s.errors.is_empty() {
            "error"
        } else {
            "idle"
        };
        let progress = if s.building {
            json!({"done":s.done,"current":s.current,"total":s.total.max(s.done+1),"percent":((s.done*100)/s.total.max(s.done+1)).min(99)})
        } else {
            Value::Null
        };
        json!({"state":state,"files":files,"folders":folders,"roots":roots,"builtAt":built,"progress":progress,"watching":s.watching,"liveUpdate":s.live,"refreshing":s.refreshing,"generation":self.generation.load(Ordering::Relaxed),"overlay":overlay,"excluded":s.excluded})
    }
    fn status_event(&self) {
        (self.emit)(json!({"event":"status","data":self.status()}));
    }
    fn search(&self, args: &Value, cancel: &dyn Fn() -> bool) -> Result<Value, Error> {
        let started = Instant::now();
        let state = self.state_name_without_data();
        let query = args["query"].as_str().unwrap_or("");
        let offset = query::integer(&args["offset"], 0)?.max(0) as usize;
        let limit = query::integer(&args["limit"], 200)?.clamp(0, 5000) as usize;
        let compiled = Compiled::new(query, args)?;
        let desc = query::truth(&args["desc"]);
        let hidden = query::truth(&args["showHidden"]);
        let sort = match args["sort"].as_str().unwrap_or("name") {
            s @ ("name" | "path" | "size" | "modified" | "type") => s,
            _ => "name",
        };
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        let Some(data) = data.as_ref() else {
            return Ok(
                json!({"total":0,"items":[],"tookMs":0.0,"offset":offset,"state":state.clone(),"generation":self.generation.load(Ordering::Relaxed),"highlight":compiled.highlight}),
            );
        };
        let mut matches = Vec::new();
        for (i, g) in data.ordered().into_iter().enumerate() {
            if i % 1024 == 0 && (cancel() || self.stopped.load(Ordering::Relaxed)) {
                return Err(ApiError::tr(
                    "cancelled",
                    crate::i18n::current_language(),
                    "index.cancelled",
                    crate::i18n::NO_PARAMS,
                ));
            }
            if compiled.test(data, g, hidden) {
                matches.push(g)
            }
        }
        match sort {
            "size" => matches.sort_by_key(|g| data.fields(*g).0),
            "modified" => matches.sort_by_key(|g| data.fields(*g).1),
            "path" => matches.sort_by_cached_key(|g| {
                let p = data.parent(*g);
                if p < 0 {
                    Vec::new()
                } else {
                    cache::lower(data.path(p as usize).as_bytes())
                }
            }),
            "type" => matches.sort_by_cached_key(|g| {
                let f = data.fields(*g).2;
                if f & DIR != 0 {
                    Vec::new()
                } else {
                    let name = data.name(*g, true);
                    name.iter()
                        .rposition(|b| *b == b'.')
                        .filter(|p| *p > 0)
                        .map(|p| name[p + 1..].to_vec())
                        .unwrap_or_default()
                }
            }),
            _ => {}
        }
        if desc {
            if sort == "name" {
                matches.reverse()
            } else {
                let key = |g: usize| match sort {
                    "size" => json!(data.fields(g).0),
                    "modified" => json!(data.fields(g).1),
                    "path" => json!(if data.parent(g) < 0 {
                        String::new()
                    } else {
                        String::from_utf8_lossy(&cache::lower(
                            data.path(data.parent(g) as usize).as_bytes(),
                        ))
                        .into_owned()
                    }),
                    _ => json!(cache::extension(
                        &cache::name_string(&data.name(g, true)),
                        data.fields(g).2 & DIR != 0
                    )),
                };
                matches.sort_by(|a, b| {
                    let (a, b) = (key(*a), key(*b));
                    if a.is_number() {
                        b.as_i64().cmp(&a.as_i64())
                    } else {
                        b.as_str().cmp(&a.as_str())
                    }
                })
            }
        }
        let total = matches.len();
        let items = matches
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|g| item(data, g))
            .collect::<Vec<_>>();
        Ok(
            json!({"total":total,"items":items,"tookMs":(started.elapsed().as_secs_f64()*10000.).round()/10.,"offset":offset,"state":state,"generation":self.generation.load(Ordering::Relaxed),"highlight":compiled.highlight}),
        )
    }
    fn state_name_without_data(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.building {
            "building"
        } else if s.ready {
            "ready"
        } else if s.configured && !s.errors.is_empty() {
            "error"
        } else {
            "idle"
        }
        .into()
    }
    fn changed(&self) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        (self.emit)(json!({"event":"changed","data":{"generation":generation}}));
    }
    fn directories(&self, paths: Vec<String>) {
        let excluded = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.excluded
                .iter()
                .cloned()
                .chain(cache::EXCLUDES.iter().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        };
        let mut guard = self.data.write().unwrap_or_else(|e| e.into_inner());
        let Some(data) = guard.as_mut() else { return };
        let mut changed = false;
        for path in paths {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
            let Some(parent) = data.lookup(&path) else {
                continue;
            };
            if data.fields(parent).2 & DIR == 0 {
                continue;
            }
            let mut pending = VecDeque::from([(parent, PathBuf::from(&path))]);
            while let Some((parent, path)) = pending.pop_front() {
                if let Ok(meta) = fs::metadata(&path) {
                    let (size, oldmt, flags, cat) = data.fields(parent);
                    let mt = meta.mtime() * 1000 + meta.mtime_nsec() / 1_000_000;
                    if mt != oldmt {
                        data.changes.insert(parent, (size, mt, flags, cat));
                        changed = true;
                    }
                }

                let si = data.segment(parent);
                let seg = &data.segments[si];
                let (dev, root, win) = (
                    seg.dev,
                    seg.path.clone(),
                    [
                        "ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "msdos", "fat", "fat32",
                    ]
                    .contains(&seg.fs.as_str()),
                );
                let mut old = data
                    .children(parent)
                    .into_iter()
                    .map(|g| (cache::name_string(&data.name(g, false)), g))
                    .collect::<HashMap<_, _>>();
                let entries = match fs::read_dir(&path) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                        continue;
                    };
                    let name = entry.file_name();
                    let text = name.to_string_lossy().into_owned();
                    if meta.is_dir()
                        && excluded.iter().any(|s| {
                            s == entry.path().to_string_lossy().as_ref()
                                && cache::beneath(s, &root)
                                && s != &root
                        })
                    {
                        continue;
                    }
                    let (mut f, c) =
                        cache::classify(&text, &meta, data.fields(parent).2 & HIDDEN != 0, win);
                    if meta.is_dir() && meta.dev() != dev {
                        f |= MOUNT
                    }
                    let size = if f & DIR != 0 || meta.file_type().is_symlink() {
                        0
                    } else {
                        meta.size().min(i64::MAX as u64) as i64
                    };
                    let mt = meta.mtime() * 1000 + meta.mtime_nsec() / 1000000;
                    if let Some(g) = old.remove(&text) {
                        let (prev, oldmt, oldf, oldc) = data.fields(g);
                        let size = if f & DIR != 0 { prev } else { size };
                        if (size, mt, f, c) != (prev, oldmt, oldf, oldc) {
                            data.changes.insert(g, (size, mt, f, c));
                            changed = true
                        }
                    } else {
                        let g = data.len();
                        data.overlay.push(Row {
                            parent: parent as i32,
                            name: cache::name_bytes(name.as_os_str().as_encoded_bytes()),
                            size,
                            modified: mt,
                            flags: f,
                            category: c,
                            segment: si,
                        });
                        changed = true;
                        if f & DIR != 0 && f & MOUNT == 0 {
                            pending.push_back((g, entry.path()));
                        }
                    }
                }
                for (_, g) in old {
                    let mut gone = VecDeque::from([g]);
                    while let Some(g) = gone.pop_front() {
                        gone.extend(data.children(g));
                        let (s, m, f, c) = data.fields(g);
                        data.changes.insert(g, (s, m, f | DEL, c));
                        changed = true
                    }
                }
            }
        }
        if changed {
            recount_sizes(data);
            data.invalidate();
        }
        drop(guard);
        if changed {
            self.setup_watches();
            self.changed();
        }
    }
    fn refresh(&self) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.refreshing = true;
        }
        let paths = {
            let d = self.data.read().unwrap_or_else(|e| e.into_inner());
            d.as_ref()
                .map(|d| {
                    (0..d.len())
                        .filter(|g| d.fields(*g).2 & (DIR | DEL | MOUNT) == DIR)
                        .map(|g| d.path(g))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        for paths in paths.chunks(256) {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
            self.directories(paths.to_vec())
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .refreshing = false;
        self.status_event();
    }
    fn setup_watches(&self) {
        let live = self.state.lock().unwrap_or_else(|e| e.into_inner()).live;
        if !live {
            *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .watching = 0;
            return;
        }
        let home = glib::home_dir().to_string_lossy().into_owned();
        let paths = {
            let data = self.data.read().unwrap_or_else(|e| e.into_inner());
            data.as_ref()
                .map(|d| {
                    (0..d.len())
                        .filter(|g| d.fields(*g).2 & (DIR | DEL | MOUNT) == DIR)
                        .filter_map(|g| {
                            let p = d.path(g);
                            cache::beneath(&p, &home).then_some(p)
                        })
                        .take(watch_budget())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let watcher = Watcher::new(paths, self.tx.clone());
        let count = watcher.as_ref().map(|w| w.count).unwrap_or(0);
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = watcher;
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watching = count;
    }
    fn persist(&self) -> io::Result<()> {
        let excluded = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .excluded
            .clone();
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = d.as_ref() {
            if !d.changes.is_empty() || !d.overlay.is_empty() {
                d.persist(&excluded)?;
            }
        }
        Ok(())
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = self.tx.send(Task::Stop);
    }
}
fn recount_sizes(data: &mut Data) {
    let mut sizes = vec![0i64; data.len()];
    for g in 0..data.len() {
        let (s, _, f, _) = data.fields(g);
        if f & (DIR | DEL) == 0 {
            sizes[g] = s
        }
    }
    for g in (0..data.len()).rev() {
        let parent = data.parent(g);
        if parent >= 0 && data.fields(g).2 & DEL == 0 {
            sizes[parent as usize] = sizes[parent as usize].saturating_add(sizes[g]);
        }
    }
    for (g, s) in sizes.into_iter().enumerate() {
        let (old, m, f, c) = data.fields(g);
        if f & DIR != 0 && old != s {
            data.changes.insert(g, (s, m, f, c));
        }
    }
}
fn watch_budget() -> usize {
    let maximum = fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(8192);
    let mut used = 0;
    for pid in fs::read_dir("/proc").into_iter().flatten().flatten() {
        if !pid
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            continue;
        }
        if let Ok(entries) = fs::read_dir(pid.path().join("fdinfo")) {
            for e in entries.flatten() {
                if let Ok(s) = fs::read_to_string(e.path()) {
                    used += s.lines().filter(|l| l.starts_with("inotify wd:")).count()
                }
            }
        }
    }
    40000.min(maximum.saturating_sub(used).saturating_sub(1024))
}
struct Watcher {
    stop: Arc<AtomicBool>,
    fd: i32,
    count: usize,
}
impl Watcher {
    fn new(paths: Vec<String>, tx: mpsc::Sender<Task>) -> Option<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return None;
        }
        let mut watches = HashMap::new();
        for path in paths {
            let Ok(p) = CString::new(path.as_bytes()) else {
                continue;
            };
            let wd = unsafe {
                libc::inotify_add_watch(
                    fd,
                    p.as_ptr(),
                    libc::IN_CREATE
                        | libc::IN_DELETE
                        | libc::IN_MOVED_FROM
                        | libc::IN_MOVED_TO
                        | libc::IN_ATTRIB
                        | libc::IN_CLOSE_WRITE
                        | libc::IN_DELETE_SELF
                        | libc::IN_MOVE_SELF
                        | libc::IN_ONLYDIR
                        | libc::IN_DONT_FOLLOW,
                )
            };
            if wd >= 0 {
                watches.insert(wd, path);
            }
        }
        let count = watches.len();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        thread::spawn(move || {
            let mut bytes = vec![0u8; 128 * 1024];
            let mut pending = HashSet::new();
            let mut changed = Instant::now();
            while !done.load(Ordering::Relaxed) {
                let mut poll = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe { libc::poll(&mut poll, 1, 100) };
                let n = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
                if n > 0 {
                    let mut at = 0;
                    while at + 16 <= n as usize {
                        let wd = i32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
                        let mask = u32::from_ne_bytes(bytes[at + 4..at + 8].try_into().unwrap());
                        let len = u32::from_ne_bytes(bytes[at + 12..at + 16].try_into().unwrap())
                            as usize;
                        if mask & libc::IN_Q_OVERFLOW != 0 {
                            let _ = tx.send(Task::Refresh);
                        }
                        if let Some(p) = watches.get(&wd) {
                            pending.insert(p.clone());
                            changed = Instant::now();
                        }
                        at += 16 + len;
                    }
                }
                if !pending.is_empty() && changed.elapsed() > Duration::from_millis(100) {
                    let _ = tx.send(Task::Directories(pending.drain().collect()));
                }
            }
            unsafe {
                libc::close(fd);
            }
        });
        Some(Self { stop, fd, count })
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.fd;
    }
}
fn item(data: &Data, g: usize) -> Value {
    let (size, modified, flags, _) = data.fields(g);
    let path = data.path(g);
    let isdir = flags & DIR != 0;
    let name = if data.parent(g) < 0 {
        Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone())
    } else {
        cache::name_string(&data.name(g, false))
    };
    let parent = if data.parent(g) < 0 {
        if path == "/" {
            String::new()
        } else {
            Path::new(path.trim_end_matches('/'))
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned()
        }
    } else {
        data.path(data.parent(g) as usize)
    };
    let ext = cache::extension(&name, isdir);
    json!({"path":path,"name":name,"dir":parent,"isDir":isdir,"size":size,"modified":modified,"ext":ext,"mime":if isdir{"inode/directory".into()}else{mime(&ext,flags)},"hidden":flags&HIDDEN!=0,"isSymlink":flags&LINK!=0})
}
fn mime(ext: &str, flags: u8) -> String {
    if flags & LINK != 0 && ext.is_empty() {
        return "inode/symlink".into();
    }
    let known = match ext {
        "desktop" => "application/x-desktop",
        "appimage" => "application/vnd.appimage",
        "sh" => "application/x-shellscript",
        "md" => "text/markdown",
        "mkv" => "video/x-matroska",
        "flac" => "audio/flac",
        "opus" => "audio/ogg",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "avif" => "image/avif",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "deb" => "application/vnd.debian.binary-package",
        "iso" => "application/x-cd-image",
        "py" => "text/x-python",
        "js" => "text/javascript",
        "json" => "application/json",
        "log" => "text/plain",
        "kdbx" => "application/x-keepass2",
        "torrent" => "application/x-bittorrent",
        "epub" => "application/epub+zip",
        "m4a" => "audio/mp4",
        "mts" | "ts" => "video/mp2t",
        "odt" => "application/vnd.oasis.opendocument.text",
        "txt" => "text/plain",
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    if !ext.is_empty() {
        if let Ok(text) = fs::read_to_string("/etc/mime.types") {
            for line in text.lines().filter(|s| !s.starts_with('#')) {
                let mut p = line.split_whitespace();
                if let Some(m) = p.next() {
                    if p.any(|e| e == ext) {
                        return m.into();
                    }
                }
            }
        }
    }
    if flags & cache::EXEC != 0 {
        "application/x-executable"
    } else {
        "application/octet-stream"
    }
    .into()
}
fn qkey(args: &Value) -> String {
    serde_json::to_string(
        &[
            "query",
            "filter",
            "matchCase",
            "matchPath",
            "regex",
            "wholeWord",
            "showHidden",
            "sort",
            "desc",
        ]
        .iter()
        .map(|k| args[*k].clone())
        .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}
pub fn main() {
    let mut cache = None;
    let args = std::env::args().collect::<Vec<_>>();
    for pair in args.windows(2) {
        if pair[0] == "--cache" {
            cache = Some(PathBuf::from(&pair[1]));
        }
    }
    let Some(cache) = cache else {
        eprintln!("Gebruik: win11-index-worker --cache MAP");
        return;
    };
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
    let _ = fs::write("/proc/self/oom_score_adj", b"500");
    let out = Arc::new(Mutex::new(io::stdout()));
    let emit: Emit = Arc::new(move |obj| {
        let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
        if serde_json::to_writer(&mut *out, &obj)
            .and_then(|_| out.write_all(b"\n").map_err(serde_json::Error::io))
            .is_err()
        {
            std::process::exit(0)
        }
        let _ = out.flush();
    });
    let engine = Engine::new(cache, emit.clone());
    let latest = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let (tx, rx) = mpsc::sync_channel::<(Value, Value)>(512);
    let search_engine = engine.clone();
    let search_latest = latest.clone();
    let search_emit = emit.clone();
    thread::spawn(move || {
        while let Ok((id, args)) = rx.recv() {
            let channel = args["channel"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("default")
                .to_string();
            let key = qkey(&args);
            let cancel = || {
                search_latest
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&channel)
                    .is_some_and(|v| v != &key)
            };
            let result = if cancel() {
                Err(ApiError::tr(
                    "cancelled",
                    crate::i18n::current_language(),
                    "index.cancelled",
                    crate::i18n::NO_PARAMS,
                ))
            } else {
                search_engine.search(&args, &cancel)
            };
            match result {
                Ok(v) => search_emit(json!({"id":id,"ok":true,"result":v})),
                Err(e) => search_emit(json!({"id":id,"ok":false,"error":e.json()})),
            }
            #[cfg(target_env = "gnu")]
            unsafe {
                libc::malloc_trim(0);
            }
        }
    });
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.len() > 1024 * 1024 {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = req["id"].clone();
        let args = if req["args"].is_object() {
            req["args"].clone()
        } else {
            json!({})
        };
        let mut shutdown = false;
        let result = match req["method"].as_str().unwrap_or("") {
            "search" => {
                let channel = args["channel"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("default")
                    .to_string();
                latest
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(channel, qkey(&args));
                if tx.send((id.clone(), args)).is_err() {
                    break;
                }
                continue;
            }
            "configure" => {
                engine.cancel.store(true, Ordering::Relaxed);
                let _ = engine.tx.send(Task::Configure(args));
                Ok(json!({"ok":true}))
            }
            "rebuild" | "compact" => {
                engine.cancel.store(true, Ordering::Relaxed);
                let _ = engine.tx.send(Task::Rebuild);
                Ok(json!({"ok":true}))
            }
            "persist" => {
                let _ = engine.tx.send(Task::Persist);
                Ok(json!({"ok":true}))
            }
            "status" => Ok(engine.status()),
            "ping" => Ok(json!({"pong":true,"pid":std::process::id()})),
            "autoRoots" => Ok(json!({"roots":cache::auto_roots(&cache::mounts())})),
            "shutdown" => {
                shutdown = true;
                Ok(json!({"ok":true}))
            }
            _ => Err(ApiError::tr(
                "unknown_method",
                crate::i18n::current_language(),
                "index.unknownCommand",
                crate::i18n::NO_PARAMS,
            )),
        };
        if !id.is_null() {
            match result {
                Ok(v) => emit(json!({"id":id,"ok":true,"result":v})),
                Err(e) => emit(json!({"id":id,"ok":false,"error":e.json()})),
            }
        }
        if shutdown {
            break;
        }
    }
    engine.stop();
    let _ = engine.persist();
}
