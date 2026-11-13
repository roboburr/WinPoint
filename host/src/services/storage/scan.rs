use super::{ServiceContext, common::*};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{self, Lang},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
#[derive(Default)]
struct Node {
    name: String,
    path: PathBuf,
    size: u64,
    files: u64,
    dirs: u64,
    parent: Option<usize>,
    children: Vec<usize>,
    top: Vec<(u64, String)>,
    rest_size: u64,
    rest_count: u64,
    depth: usize,
}
// The same min-heap push/sift operations as heapq retain its tie order.
fn push<T: Ord>(heap: &mut Vec<T>, item: T) {
    heap.push(item);
    let mut pos = heap.len() - 1;
    while pos > 0 {
        let parent = (pos - 1) >> 1;
        if heap[pos] < heap[parent] {
            heap.swap(pos, parent);
            pos = parent
        } else {
            break;
        }
    }
}
fn replace<T: Ord>(heap: &mut Vec<T>, item: T) -> T {
    let old = std::mem::replace(&mut heap[0], item);
    let mut pos = 0;
    let end = heap.len();
    while 2 * pos + 1 < end {
        let left = 2 * pos + 1;
        let right = left + 1;
        let child = if right < end && heap[right] <= heap[left] {
            right
        } else {
            left
        };
        heap.swap(pos, child);
        pos = child;
    }
    while pos > 0 {
        let parent = (pos - 1) >> 1;
        if heap[pos] < heap[parent] {
            heap.swap(pos, parent);
            pos = parent
        } else {
            break;
        }
    }
    old
}
fn random_hex(bytes: usize) -> String {
    let mut data = vec![0u8; bytes];
    let got = unsafe { libc::getrandom(data.as_mut_ptr() as *mut _, data.len(), 0) };
    if got < 0 {
        let seed = (wall() * 1e9) as u128 ^ std::process::id() as u128;
        for (i, b) in data.iter_mut().enumerate() {
            *b = (seed.rotate_left((i * 8) as u32) & 255) as u8;
        }
    }
    data.iter().map(|b| format!("{b:02x}")).collect()
}
#[derive(Clone, Default)]
struct Progress {
    bytes: u64,
    files: u64,
    current: PathBuf,
}
struct Job {
    id: String,
    path: PathBuf,
    max_depth: usize,
    top_n: usize,
    started: f64,
    cancel: AtomicBool,
    done: AtomicBool,
    state: Mutex<JobState>,
}
#[derive(Default)]
struct JobState {
    progress: Progress,
    tree: Option<Value>,
    /// Kept as the error itself (code, key + params): its message is rendered when it is read (round2-gate §2).
    error: Option<ApiError>,
    large: Vec<(u64, String, i64)>,
    skipped: u64,
}
#[derive(Clone)]
struct Cached {
    tree: Value,
    time: f64,
}
#[derive(Default)]
struct State {
    jobs: HashMap<String, Arc<Job>>,
    cached_ids: VecDeque<(String, (PathBuf, usize, usize))>,
    cache: HashMap<(PathBuf, usize, usize), Cached>,
    large: HashMap<PathBuf, (f64, Vec<(u64, String, i64)>)>,
}
pub(super) struct Scanner {
    ctx: Arc<ServiceContext>,
    state: Mutex<State>,
    stopped: AtomicBool,
}
impl Scanner {
    pub fn new(ctx: Arc<ServiceContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            state: Mutex::new(State::default()),
            stopped: AtomicBool::new(false),
        })
    }
    pub fn start(
        self: &Arc<Self>,
        path: &Value,
        max_depth: &Value,
        top_n: &Value,
        refresh: bool,
    ) -> ApiResult {
        let path = check_dir(path, self.ctx.lang())?;
        let md = if max_depth.is_null() {
            4
        } else {
            valint(max_depth).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "storage.badArgs",
                    crate::i18n::NO_PARAMS,
                )
            })?
        }
        .clamp(1, 8) as usize;
        let tn = if top_n.is_null() {
            40
        } else {
            valint(top_n).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "storage.badArgs",
                    crate::i18n::NO_PARAMS,
                )
            })?
        }
        .clamp(5, 200) as usize;
        let key = (path.clone(), md, tn);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !refresh {
            if let Some(c) = state.cache.get(&key).cloned() {
                let id = format!("cached-{}", random_hex(4));
                state.cached_ids.push_back((id.clone(), key));
                while state.cached_ids.len() > 64 {
                    state.cached_ids.pop_front();
                }
                let ctx = self.ctx.clone();
                let late_id = id.clone();
                let late_path = path.clone();
                let mut late_tree = c.tree.clone();
                let at = c.time;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(150));
                    localize(&mut late_tree, &ctx.language());
                    let mut event = done_event(&late_id, Some(&late_tree));
                    event["path"] = json!(late_path);
                    event["cached"] = json!(true);
                    event["scannedAt"] = json!((at * 1000.) as i64);
                    ctx.emit("storage.scanDone", event);
                });
                let mut tree = c.tree;
                localize(&mut tree, &self.ctx.language());
                return Ok(
                    json!({"jobId":id,"cached":true,"tree":tree,"scannedAt":(c.time*1000.)as i64}),
                );
            }
        }
        for j in state.jobs.values() {
            if !j.done.load(Ordering::Relaxed)
                && j.path == path
                && j.max_depth == md
                && j.top_n == tn
            {
                return Ok(json!({"jobId":j.id,"running":true}));
            }
        }
        if state
            .jobs
            .values()
            .filter(|j| !j.done.load(Ordering::Relaxed))
            .count()
            >= 3
        {
            return Err(ApiError::tr(
                "busy",
                self.ctx.lang(),
                "storage.busy",
                crate::i18n::NO_PARAMS,
            ));
        }
        static ID: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "scan-{}-{}",
            ID.fetch_add(1, Ordering::Relaxed),
            random_hex(3)
        );
        let job = Arc::new(Job {
            id: id.clone(),
            path: path.clone(),
            max_depth: md,
            top_n: tn,
            started: now(),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
            state: Mutex::new(JobState {
                progress: Progress {
                    current: path,
                    ..Default::default()
                },
                ..Default::default()
            }),
        });
        state.jobs.insert(id.clone(), job.clone());
        drop(state);
        let scanner = Arc::downgrade(self);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            scan_priority();
            let result = walk(&job, &ctx);
            {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok((nodes, progress, large, skipped)) => {
                        state.progress = progress;
                        state.large = large;
                        state.skipped = skipped;
                        if job.cancel.load(Ordering::Relaxed) {
                            state.error = Some(ApiError::tr(
                                "cancelled",
                                ctx.lang(),
                                "storage.scanStopped",
                                crate::i18n::NO_PARAMS,
                            ));
                        } else {
                            state.tree = Some(serialize(
                                &nodes,
                                job.top_n,
                                job.max_depth,
                                skipped,
                                &ctx.language(),
                            ));
                        }
                    }
                    Err(e) => state.error = Some(e),
                }
            }
            let delay = 0.15 - (now() - job.started);
            if delay > 0. {
                std::thread::sleep(Duration::from_secs_f64(delay));
            }
            job.done.store(true, Ordering::Release);
            if let Some(scanner) = scanner.upgrade() {
                scanner.done(&job);
            }
            let state = job.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut event = done_event(&job.id, state.tree.as_ref());
            event["path"] = json!(job.path);
            event["bytes"] = json!(state.progress.bytes);
            event["files"] = json!(state.progress.files);
            event["tookMs"] = json!(((now() - job.started) * 1000.) as i64);
            if let Some(error) = &state.error {
                event["error"] = json!(message(error, ctx.lang()));
                event["errorCode"] = json!(error.code);
            }
            ctx.emit("storage.scanDone", event);
        });
        Ok(json!({"jobId":id}))
    }
    fn done(&self, job: &Arc<Job>) {
        let result = job.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tree) = &result.tree {
            state.cache.insert(
                (job.path.clone(), job.max_depth, job.top_n),
                Cached {
                    tree: tree.clone(),
                    time: wall(),
                },
            );
            if state.cache.len() > 8 {
                if let Some(key) = state
                    .cache
                    .iter()
                    .min_by(|a, b| a.1.time.total_cmp(&b.1.time))
                    .map(|(k, _)| k.clone())
                {
                    state.cache.remove(&key);
                }
            }
            let mut large = result.large.clone();
            large.sort_by(|a, b| b.cmp(a));
            state.large.insert(job.path.clone(), (wall(), large));
            if state.large.len() > 8 {
                if let Some(key) = state
                    .large
                    .iter()
                    .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                    .map(|(k, _)| k.clone())
                {
                    state.large.remove(&key);
                }
            }
        }
        let cutoff = now() - 600.;
        state
            .jobs
            .retain(|_, j| !(j.done.load(Ordering::Relaxed) && j.started < cutoff));
    }
    pub fn cancel(&self, id: &Value) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(job) = state.jobs.get(&ps(id)) else {
            return json!({"ok":false});
        };
        job.cancel.store(true, Ordering::Relaxed);
        json!({"ok":true})
    }
    pub fn result(&self, id: &Value) -> ApiResult {
        let id = ps(id);
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = state.jobs.get(&id).cloned() {
            drop(state);
            let s = job.state.lock().unwrap_or_else(|e| e.into_inner());
            let status = if !job.done.load(Ordering::Acquire) {
                "running"
            } else if s.error.is_some() {
                "error"
            } else {
                "done"
            };
            let mut out = json!({"jobId":job.id,"state":status,"progress":{"bytes":s.progress.bytes,"files":s.progress.files,"current":s.progress.current},"path":job.path});
            if status == "done" {
                let mut tree = json!(s.tree);
                localize(&mut tree, &self.ctx.language());
                out["tree"] = tree
            }
            if let Some(e) = &s.error {
                out["error"] = json!(message(e, self.ctx.lang()));
                out["errorCode"] = json!(e.code)
            }
            return Ok(out);
        }
        if let Some((_, key)) = state.cached_ids.iter().find(|(jid, _)| jid == &id) {
            if let Some(c) = state.cache.get(key) {
                let mut tree = c.tree.clone();
                localize(&mut tree, &self.ctx.language());
                return Ok(
                    json!({"jobId":id,"state":"done","path":key.0,"tree":tree,"cached":true,"scannedAt":(c.time*1000.)as i64,"progress":{"bytes":c.tree["size"].as_u64().unwrap_or(0),"files":c.tree["files"].as_u64().unwrap_or(0),"current":key.0}}),
                );
            }
        }
        Err(ApiError::tr(
            "not_found",
            self.ctx.lang(),
            "storage.unknownScan",
            crate::i18n::NO_PARAMS,
        ))
    }
    pub fn large_for(&self, path: &Path) -> Option<(Vec<(u64, String, i64)>, bool)> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (root, (_, list)) = state
            .large
            .iter()
            .filter(|(root, _)| path == root.as_path() || path.starts_with(root))
            .max_by_key(|(root, _)| root.as_os_str().len())?;
        if path == root {
            return Some((list.clone(), true));
        }
        let full = list.len() >= 500;
        Some((
            list.iter()
                .filter(|(_, fp, _)| Path::new(fp).starts_with(path))
                .cloned()
                .collect(),
            !full,
        ))
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        for j in self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .values()
        {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn walk(
    job: &Job,
    ctx: &Arc<ServiceContext>,
) -> Result<(Vec<Node>, Progress, Vec<(u64, String, i64)>, u64), ApiError> {
    let metadata = fs::metadata(&job.path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            ApiError::tr(
                "denied",
                ctx.lang(),
                "storage.folderDenied",
                crate::i18n::NO_PARAMS,
            )
        } else {
            ApiError::tr(
                "not_found",
                ctx.lang(),
                "storage.folderGone",
                crate::i18n::NO_PARAMS,
            )
        }
    })?;
    if !metadata.is_dir() {
        return Err(ApiError::tr(
            "not_found",
            ctx.lang(),
            "storage.folderGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    fs::read_dir(&job.path).map_err(|_| {
        ApiError::tr(
            "denied",
            ctx.lang(),
            "storage.folderDenied",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let dev = metadata.dev();
    let mut nodes = vec![Node {
        name: job
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into()),
        path: job.path.clone(),
        size: metadata.blocks() * 512,
        ..Default::default()
    }];
    let mut seen = HashSet::new();
    let mut stack = vec![(job.path.clone(), 0usize, 0usize)];
    let mut last = now();
    let mut progress = Progress {
        current: job.path.clone(),
        ..Default::default()
    };
    let mut large = vec![];
    let mut skipped = 0;
    let mut count = 0;
    while let Some((path, node, depth)) = stack.pop() {
        if job.cancel.load(Ordering::Relaxed) {
            break;
        }
        progress.current = path.clone();
        let Ok(entries) = fs::read_dir(&path) else {
            skipped += 1;
            continue;
        };
        let (mut local, mut files, mut dirs) = (0u64, 0u64, 0u64);
        let real = nodes[node].depth == depth;
        for e in entries.flatten() {
            let full = e.path();
            let Ok(s) = fs::symlink_metadata(&full) else {
                continue;
            };
            let size = s.blocks() * 512;
            if s.is_dir() {
                if s.dev() != dev
                    || ["/proc", "/sys", "/dev", "/run"].contains(&full.to_string_lossy().as_ref())
                {
                    continue;
                }
                local += size;
                dirs += 1;
                if real && depth < job.max_depth {
                    let child = nodes.len();
                    nodes.push(Node {
                        name: e.file_name().to_string_lossy().into_owned(),
                        path: full.clone(),
                        size,
                        parent: Some(node),
                        depth: depth + 1,
                        ..Default::default()
                    });
                    nodes[node].children.push(child);
                    stack.push((full, child, depth + 1));
                } else {
                    stack.push((full, node, depth + 1));
                }
            } else {
                if s.nlink() > 1 && !seen.insert((s.dev(), s.ino())) {
                    continue;
                }
                local += size;
                files += 1;
                if real {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let n = &mut nodes[node];
                    if n.top.len() < job.top_n {
                        push(&mut n.top, (size, name));
                    } else if size > n.top[0].0 {
                        let old = replace(&mut n.top, (size, name));
                        n.rest_size += old.0;
                        n.rest_count += 1;
                    } else {
                        n.rest_size += size;
                        n.rest_count += 1;
                    }
                }
                if size >= 1048576 {
                    let record = (
                        size,
                        full.to_string_lossy().into_owned(),
                        s.mtime() * 1000 + s.mtime_nsec() / 1000000,
                    );
                    if large.len() < 500 {
                        push(&mut large, record)
                    } else if size > large[0].0 {
                        replace(&mut large, record);
                    }
                }
            }
            count += 1;
            if count & 2047 == 0 {
                if job.cancel.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::yield_now();
            }
        }
        let mut current = Some(node);
        while let Some(i) = current {
            nodes[i].size += local;
            nodes[i].files += files;
            nodes[i].dirs += dirs;
            current = nodes[i].parent;
        }
        progress.bytes += local;
        progress.files += files;
        *job.state.lock().unwrap_or_else(|e| e.into_inner()) = JobState {
            progress: progress.clone(),
            ..Default::default()
        };
        if now() - last >= 0.25 {
            last = now();
            ctx.emit("storage.scanProgress",json!({"jobId":job.id,"bytes":progress.bytes,"files":progress.files,"current":path}));
        }
    }
    progress.bytes = nodes[0].size;
    Ok((nodes, progress, large, skipped))
}
fn serialize(nodes: &[Node], top_n: usize, max_depth: usize, skipped: u64, lang: &str) -> Value {
    #[derive(Clone)]
    enum Item {
        Dir(usize),
        File(u64, String),
        Other(u64, u64, u64),
    }
    let mut layouts = HashMap::<usize, (Vec<Item>, Option<bool>)>::new();
    let mut queue = VecDeque::from([0]);
    let mut budget = 8000i64;
    while let Some(i) = queue.pop_front() {
        let n = &nodes[i];
        let mut items = n
            .children
            .iter()
            .map(|c| (nodes[*c].size, Item::Dir(*c)))
            .collect::<Vec<_>>();
        items.extend(
            n.top
                .iter()
                .map(|(s, name)| (*s, Item::File(*s, name.clone()))),
        );
        items.sort_by(|a, b| b.0.cmp(&a.0));
        if budget <= 0 {
            layouts.insert(i, (vec![], Some(!items.is_empty())));
            continue;
        }
        let mut children = vec![];
        for (_, item) in items.iter().take(top_n) {
            if let Item::Dir(c) = item {
                if nodes[*c].depth < max_depth {
                    queue.push_back(*c)
                }
            }
            children.push(item.clone());
        }
        budget -= children.len() as i64;
        let rest = items.iter().skip(top_n);
        let rest_size = rest.clone().map(|i| i.0).sum::<u64>() + n.rest_size;
        let rest_count = items.len().saturating_sub(top_n) as u64 + n.rest_count;
        let rest_files = rest
            .map(|(_, v)| match v {
                Item::Dir(c) => nodes[*c].files,
                _ => 1,
            })
            .sum::<u64>()
            + n.rest_count;
        if rest_count > 0 {
            children.push(Item::Other(rest_size, rest_count, rest_files));
        }
        layouts.insert(i, (children, None));
    }
    fn build(
        i: usize,
        nodes: &[Node],
        layouts: &HashMap<usize, (Vec<Item>, Option<bool>)>,
        max_depth: usize,
        lang: &str,
    ) -> Value {
        let n = &nodes[i];
        let mut v = json!({"name":n.name,"path":n.path,"size":n.size,"files":n.files,"dirs":n.dirs,"isDir":true,"children":[]});
        if let Some((items, truncated)) = layouts.get(&i) {
            if let Some(flag) = truncated {
                v["truncated"] = json!(*flag)
            }
            v["children"]=json!(items.iter().map(|item|match item{Item::Dir(c)=>{let mut child=build(*c,nodes,layouts,max_depth,lang);if nodes[*c].depth>=max_depth&&(nodes[*c].dirs>0||nodes[*c].files>0){child["truncated"]=json!(true)}child},Item::File(size,name)=>json!({"name":name,"path":n.path.join(name),"size":size,"files":1,"isDir":false}),Item::Other(size,count,files)=>json!({"name":tr(lang,"other",&[]),"other":true,"path":n.path,"size":size,"files":files,"count":count,"isDir":false})}).collect::<Vec<_>>());
        }
        v
    }
    let mut root = build(0, nodes, &layouts, max_depth, lang);
    if skipped > 0 {
        root["skipped"] = json!(skipped)
    }
    root
}
/// The message of a stored scan error in `lang`: a catalogue error is rendered again from its key + params, so a
/// result read after a language switch is in the new language.
fn message(error: &ApiError, lang: Lang) -> String {
    match &error.key {
        Some(key) => i18n::trf(lang, key, error.params.as_ref().unwrap_or(i18n::NO_PARAMS)),
        None => error.message.clone(),
    }
}
/// The "other items" rows of a scan tree in `lang`. A tree is cached for later scans (keyed by path, depth and
/// size, not by language), so its one text is set again whenever a tree is handed out.
fn localize(tree: &mut Value, lang: &str) {
    if tree["other"] == true {
        tree["name"] = json!(tr(lang, "other", &[]));
    }
    if let Some(children) = tree.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            localize(child, lang);
        }
    }
}
pub(super) fn done_event(id: &str, tree: Option<&Value>) -> Value {
    let mut out = json!({"jobId":id,"done":true});
    if let Some(tree) = tree {
        if tree.to_string().chars().count() <= 16 * 1024 {
            out["tree"] = tree.clone()
        } else {
            out["hasTree"] = json!(true)
        }
    } else {
        out["tree"] = Value::Null
    }
    out
}
pub(super) fn large(scanner: &Scanner, path: &Value, limit: &Value, min_size: &Value) -> ApiResult {
    let lang = scanner.ctx.lang();
    let path = check_dir(path, lang)?;
    let lim = if limit.is_null() {
        50
    } else {
        valint(limit).ok_or_else(|| {
            ApiError::tr("bad_args", lang, "storage.badArgs", crate::i18n::NO_PARAMS)
        })?
    }
    .clamp(1, 500) as usize;
    let min = if !truth(min_size) {
        0
    } else {
        valint(min_size)
            .ok_or_else(|| {
                ApiError::tr("bad_args", lang, "storage.badArgs", crate::i18n::NO_PARAMS)
            })?
            .max(0)
    } as u64;
    let cached = scanner
        .large_for(&path)
        .filter(|(v, complete)| v.len() >= lim || *complete && min >= 1048576)
        .map(|v| v.0);
    let mut values = if let Some(v) = cached {
        v
    } else {
        let dev = fs::metadata(&path)
            .map_err(|_| {
                ApiError::tr(
                    "not_found",
                    lang,
                    "storage.noSuchFolder",
                    crate::i18n::NO_PARAMS,
                )
            })?
            .dev();
        let mut found = vec![];
        let deadline = now() + 4.;
        let mut stack = vec![path.clone()];
        let mut n = 0;
        while !stack.is_empty() && now() < deadline {
            let dir = stack.pop().unwrap();
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for e in entries.flatten() {
                let full = e.path();
                let Ok(s) = fs::symlink_metadata(&full) else {
                    continue;
                };
                if s.is_dir() {
                    if s.dev() == dev
                        && !["/proc", "/sys", "/dev", "/run"]
                            .contains(&full.to_string_lossy().as_ref())
                    {
                        stack.push(full);
                    }
                    continue;
                }
                let size = s.blocks() * 512;
                if size < min.max(1) {
                    continue;
                }
                let item = (
                    size,
                    full.to_string_lossy().into_owned(),
                    s.mtime() * 1000 + s.mtime_nsec() / 1000000,
                );
                if found.len() < lim {
                    push(&mut found, item)
                } else if size > found[0].0 {
                    replace(&mut found, item);
                }
                n += 1;
                if n > 300000 {
                    stack.clear();
                    break;
                }
            }
        }
        found.sort_by(|a, b| b.cmp(a));
        found
    };
    let mut out = vec![];
    for (size, path, modified) in values.drain(..) {
        if size < min {
            continue;
        }
        let name = Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mime = crate::files::filename_mime(&name);
        let icon = mime_icon(&mime);
        out.push(json!({"path":path,"name":name,"size":size,"modified":modified,"mime":mime,"icon":icon}));
        if out.len() >= lim {
            break;
        }
    }
    Ok(json!(out))
}
fn mime_icon(mime: &str) -> String {
    use gio::prelude::*;
    let mut names = vec![];
    let themed = gio::content_type_get_icon(mime);
    if let Some(icon) = themed.downcast_ref::<gio::ThemedIcon>() {
        names.extend(icon.names().iter().map(|s| s.to_string()));
    }
    if let Some(name) = gio::content_type_get_generic_icon_name(mime) {
        names.push(name.to_string())
    }
    let major = mime.split('/').next().unwrap_or("");
    names.extend([
        mime.replace('/', "-"),
        format!("{major}-x-generic"),
        if major == "text" {
            "text-x-generic"
        } else {
            "application-x-generic"
        }
        .into(),
        "unknown".into(),
    ]);
    names.sort_by_key(|s| s.ends_with("-symbolic"));
    names
        .iter()
        .find(|s| crate::files::icon_exists(s))
        .cloned()
        .unwrap_or_else(|| "text-x-generic".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    // R2-B: a stored scan error and a cached tree are rendered when they are read (round2-gate §2)
    #[test]
    fn stored_errors_and_cached_trees_follow_the_language() {
        let stopped = ApiError::tr(
            "cancelled",
            Lang::Nl,
            "storage.scanStopped",
            i18n::NO_PARAMS,
        );
        assert_eq!(stopped.message, "Het scannen is gestopt.");
        assert_eq!(message(&stopped, Lang::En), "The scan was stopped.");
        assert_eq!(message(&stopped, Lang::Nl), "Het scannen is gestopt.");
        assert_eq!(message(&ApiError::new("x", "raw"), Lang::En), "raw");
        let mut tree = json!({"name":"root","children":[
            {"name":"a","children":[{"name":"Overige","other":true}]},
            {"name":"Overige","other":true}]});
        localize(&mut tree, "en");
        assert_eq!(tree["children"][0]["children"][0]["name"], "Other");
        assert_eq!(tree["children"][1]["name"], "Other");
        assert_eq!(tree["children"][0]["name"], "a");
        localize(&mut tree, "nl");
        assert_eq!(tree["children"][1]["name"], "Overige");
    }
    #[test]
    fn scan_event_avoids_broadcasting_huge_trees() {
        assert_eq!(
            done_event("scan", Some(&json!({"children":[]})))["tree"],
            json!({"children":[]})
        );
        assert_eq!(
            done_event("scan", Some(&json!({"name":"x".repeat(17000)})))["hasTree"],
            true
        );
    }
    #[test]
    fn heap_equal_sizes_retain_original_minheap_ties() {
        let mut h = vec![];
        for v in [(5, "z"), (5, "a"), (10, "c"), (2, "x")] {
            push(&mut h, v)
        }
        assert_eq!(h[0], (2, "x"));
        let old = replace(&mut h, (9, "b"));
        assert_eq!(old, (2, "x"));
        assert_eq!(h[0], (5, "a"));
    }
    #[test]
    fn unicode_tree_event_limit_counts_reference_characters() {
        let tree = json!({"name":"é".repeat(10000)});
        assert!(tree.to_string().len() > 16 * 1024);
        assert_eq!(done_event("scan", Some(&tree))["tree"], tree);
    }
    #[test]
    fn exhausted_node_budget_retains_false_truncated_flag_for_empty_dirs() {
        let mut nodes = vec![Node {
            name: "root".into(),
            path: PathBuf::from("/fixture"),
            ..Default::default()
        }];
        for i in 0..200 {
            let top = if i < 39 {
                (0..200).map(|j| (1024, format!("file-{j}"))).collect()
            } else {
                vec![]
            };
            let index = nodes.len();
            nodes[0].children.push(index);
            nodes.push(Node {
                name: format!("child-{i}"),
                path: PathBuf::from(format!("/fixture/child-{i}")),
                parent: Some(0),
                depth: 1,
                top,
                ..Default::default()
            });
        }
        let tree = serialize(&nodes, 200, 2, 0, "nl");
        assert_eq!(
            tree["children"][38]["children"].as_array().unwrap().len(),
            200
        );
        assert_eq!(tree["children"][39]["truncated"], false);
        assert!(
            tree["children"][39]["children"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
