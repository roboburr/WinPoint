use super::{ServiceContext, common::*};
use crate::{
    files::{ApiError, ApiResult},
    i18n::Lang,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub(super) struct Cleanup {
    ctx: Arc<ServiceContext>,
    cache: Mutex<Option<(f64, String, Vec<Value>)>>,
}
impl Cleanup {
    pub fn new(ctx: Arc<ServiceContext>) -> Self {
        Self {
            ctx,
            cache: Mutex::new(None),
        }
    }
    pub fn list(&self, refresh: bool) -> Vec<Value> {
        let lang = self.ctx.language();
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((at, old_lang, values)) = &*cache {
                if !refresh && now() - at < 60. && old_lang == &lang {
                    return values.clone();
                }
            }
        }
        let home = canonical(&home());
        let deadline = Some(now() + 20.);
        let mut values = vec![];
        let mut add = |id: &str,
                       size: Option<u64>,
                       admin: bool,
                       safe: bool,
                       default: bool,
                       details: Value,
                       available: bool,
                       description: Option<String>| {
            values.push(json!({"id":id,"name":tr(&lang,&format!("k_{id}"),&[]),"description":description.unwrap_or_else(||tr(&lang,&format!("d_{id}"),&[])),"size":size,"needsAdmin":admin,"safe":safe,"defaultSelected":default&&available&&size.is_none_or(|s|s>0),"details":details,"available":available}));
        };
        let trash = trash_size();
        add(
            "trash",
            trash["size"].as_u64(),
            false,
            true,
            false,
            json!({"count":trash["count"]}),
            true,
            None,
        );
        let thumbs = du(&thumbnail_dir(), deadline, true).0
            + du(&self.ctx.cache_dir.join("thumbs"), deadline, true).0;
        add(
            "thumbnails",
            Some(thumbs),
            false,
            true,
            true,
            json!({}),
            true,
            None,
        );
        let mut caches = safe_cache_paths(&home)
            .into_iter()
            .map(|(path, name)| json!({"name":name,"path":path,"size":du(&path,deadline,true).0}))
            .collect::<Vec<_>>();
        caches.sort_by_key(|v| std::cmp::Reverse(v["size"].as_u64().unwrap_or(0)));
        add(
            "user-cache-safe",
            Some(caches.iter().map(|v| v["size"].as_u64().unwrap_or(0)).sum()),
            false,
            true,
            true,
            json!({"items":caches}),
            true,
            None,
        );
        let running = browser_running();
        let bitems=browser_paths(&home,&lang).into_iter().map(|(key,name,path)|json!({"browser":key,"name":name,"path":path,"size":du(&path,deadline,true).0,"running":running.contains(&key)})).collect::<Vec<_>>();
        let cleanable = bitems
            .iter()
            .filter(|b| !truth(&b["running"]))
            .map(|b| b["size"].as_u64().unwrap_or(0))
            .sum();
        let busy = bitems
            .iter()
            .filter(|b| truth(&b["running"]) && b["size"].as_u64().unwrap_or(0) > 0)
            .filter_map(|b| b["name"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut desc = tr(&lang, "d_browser-cache", &[]);
        if !busy.is_empty() {
            desc.push_str(&tr(
                &lang,
                "d_browser-running",
                &[("names", busy.join(" / "))],
            ));
        }
        add(
            "browser-cache",
            Some(cleanable),
            false,
            true,
            true,
            json!({"items":bitems,"running":busy,"total":bitems.iter().map(|b|b["size"].as_u64().unwrap_or(0)).sum::<u64>()}),
            !bitems.is_empty(),
            Some(desc),
        );
        let old = old_downloads();
        add(
            "old-downloads",
            Some(old.iter().map(|v| v["size"].as_u64().unwrap_or(0)).sum()),
            false,
            false,
            false,
            json!({"items":old.iter().take(300).collect::<Vec<_>>(),"count":old.len(),"days":90,"toTrash":true}),
            true,
            None,
        );
        add(
            "apt-cache",
            Some(if Path::new("/var/cache/apt").is_dir() {
                apt_size()
            } else {
                0
            }),
            true,
            true,
            false,
            json!({}),
            command_exists("apt-get"),
            None,
        );
        let snaps = snap_disabled();
        add(
            "old-snaps",
            Some(snaps.iter().map(|s| s["size"].as_u64().unwrap_or(0)).sum()),
            true,
            true,
            false,
            json!({"items":snaps}),
            command_exists("snap"),
            None,
        );
        let (est, total) = journal_estimate();
        add(
            "journal",
            est,
            true,
            true,
            false,
            json!({"total":total,"keepDays":14,"estimate":true}),
            command_exists("journalctl"),
            None,
        );
        add(
            "flatpak-unused",
            None,
            false,
            true,
            false,
            json!({"scope":"user"}),
            command_exists("flatpak") && home.join(".local/share/flatpak/runtime").is_dir(),
            None,
        );
        let crash = crash_files();
        add(
            "crash-reports",
            Some(crash.iter().map(|p| du(p, None, true).0).sum()),
            false,
            true,
            true,
            json!({"count":crash.len()}),
            Path::new("/var/crash").is_dir(),
            None,
        );
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((now(), lang, values.clone()));
        values
    }
    pub fn run(&self, ids: &Value, paths: &Value) -> ApiResult {
        let lang = self.ctx.lang();
        let ids = if let Some(s) = ids.as_str() {
            vec![json!(s)]
        } else if let Some(a) = ids.as_array() {
            a.clone()
        } else {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "storage.nothingChosen",
                crate::i18n::NO_PARAMS,
            ));
        };
        let valid = data()["_CLEANUP_IDS"].as_array().unwrap();
        if ids.is_empty() || ids.iter().any(|id| !valid.contains(id)) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "storage.nothingChosen",
                crate::i18n::NO_PARAMS,
            ));
        }
        let home = canonical(&home());
        let mut values = vec![];
        let mut steps = vec![];
        let mut admin_ids = vec![];
        let mut before = BTreeMap::new();
        let mut seen = HashSet::new();
        for id in ids.iter().filter_map(Value::as_str) {
            if !seen.insert(id) {
                continue;
            }
            match id {
                "trash" => {
                    let before = trash_size()["size"].as_u64().unwrap_or(0);
                    let result = self.ctx.call("files.emptyTrash", &json!({}));
                    match result{Ok(result)=>{let after=trash_size()["size"].as_u64().unwrap_or(0);let failed=result["failed"].as_u64().unwrap_or(0);values.push(res(id,before as i128-after as i128,failed,lang));},Err(_)=>values.push(json!({"id":id,"ok":false,"freed":0,"error":crate::i18n::tr(lang, "storage.trashFailed")}))}
                }
                "thumbnails" => {
                    let mut freed = 0i128;
                    let mut errors = 0;
                    for path in [thumbnail_dir(), self.ctx.cache_dir.join("thumbs")] {
                        let before = du(&path, None, true).0;
                        errors += clear_contents(&path, true).len() as u64;
                        freed += before as i128 - du(&path, None, true).0 as i128;
                    }
                    values.push(res(id, freed, errors, lang));
                }
                "user-cache-safe" => {
                    let mut freed = 0i128;
                    let mut errors = 0;
                    for (path, _) in safe_cache_paths(&home) {
                        let before = du(&path, None, true).0;
                        errors += clear_contents(&path, false).len() as u64;
                        freed += before as i128 - du(&path, None, true).0 as i128;
                    }
                    values.push(res(id, freed, errors, lang));
                }
                "browser-cache" => {
                    let running = browser_running();
                    let mut freed = 0i128;
                    let mut errors = 0;
                    let mut skipped = BTreeSet::new();
                    for (key, name, path) in browser_paths(&home, &self.ctx.language()) {
                        if running.contains(&key) {
                            skipped.insert(name);
                            continue;
                        }
                        let before = du(&path, None, true).0;
                        errors += clear_contents(&path, false).len() as u64;
                        freed += before as i128 - du(&path, None, true).0 as i128;
                    }
                    let mut value = res(id, freed, errors, lang);
                    if !skipped.is_empty() {
                        let names = skipped.into_iter().collect::<Vec<_>>();
                        value["skipped"] = json!(names);
                        if freed == 0 {
                            value["ok"] = json!(false);
                            value["error"] = json!(crate::i18n::trf(
                                lang,
                                "storage.closeFirst",
                                &serde_json::json!({"names": (names.join(" / ")).to_string()})
                            ));
                        }
                    }
                    values.push(value);
                }
                "old-downloads" => values.push(self.trash_downloads(paths)),
                "apt-cache" => {
                    before.insert(id.to_owned(), Some(apt_size()));
                    steps.push("apt".into());
                    admin_ids.push(id.to_owned());
                }
                "old-snaps" => {
                    let snaps = snap_disabled();
                    before.insert(
                        id.to_owned(),
                        Some(snaps.iter().map(|s| s["size"].as_u64().unwrap_or(0)).sum()),
                    );
                    for s in &snaps {
                        steps.push(format!(
                            "snap:{}={}",
                            s["name"].as_str().unwrap_or(""),
                            s["revision"].as_str().unwrap_or("")
                        ));
                    }
                    if snaps.is_empty() {
                        values.push(json!({"id":id,"ok":true,"freed":0,"error":null}));
                    } else {
                        admin_ids.push(id.to_owned());
                    }
                }
                "journal" => {
                    before.insert(id.to_owned(), journal_estimate().1);
                    steps.push("journal".into());
                    admin_ids.push(id.to_owned());
                }
                "flatpak-unused" => {
                    if !command_exists("flatpak") {
                        values.push(json!({"id":id,"ok":false,"freed":0,"error":crate::i18n::tr(lang, "storage.noFlatpak")}));
                        continue;
                    }
                    let path = home.join(".local/share/flatpak");
                    let before = du(&path, Some(now() + 30.), true).0;
                    let r = run(
                        "flatpak",
                        &["uninstall", "--unused", "--user", "-y", "--noninteractive"],
                        600.,
                    );
                    let after = du(&path, Some(now() + 30.), true).0;
                    values.push(json!({"id":id,"ok":r.ok,"freed":before.saturating_sub(after),"error":if r.ok{None}else{Some(crate::i18n::tr(lang, "storage.flatpakFailed"))}}));
                }
                "crash-reports" => {
                    let mut freed = 0;
                    let mut errors = 0;
                    for path in crash_files() {
                        match fs::symlink_metadata(&path)
                            .and_then(|s| fs::remove_file(&path).map(|_| s.blocks() * 512))
                        {
                            Ok(bytes) => freed += bytes,
                            Err(_) => errors += 1,
                        }
                    }
                    values.push(res(id, freed as i128, errors, lang));
                }
                _ => {}
            }
        }
        if !steps.is_empty() {
            values.extend(run_admin(&steps, &admin_ids, &before, lang));
        }
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let freed = values
            .iter()
            .map(|v| v["freed"].as_u64().unwrap_or(0))
            .sum::<u64>();
        if ids.iter().any(|i| i == "trash" || i == "old-downloads") {
            self.ctx.emit("files.trashChanged", json!({}));
        }
        Ok(json!({"results":values,"freed":freed}))
    }
    fn trash_downloads(&self, paths: &Value) -> Value {
        let old = old_downloads();
        let allowed = old
            .iter()
            .filter_map(|o| o["path"].as_str().map(|p| (p.to_owned(), o.clone())))
            .collect::<BTreeMap<_, _>>();
        let chosen = if paths.is_null() {
            old.iter()
                .filter_map(|o| o["path"].as_str())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        } else if let Some(paths) = paths.as_array() {
            paths
                .iter()
                .filter_map(Value::as_str)
                .filter(|p| allowed.contains_key(*p))
                .map(str::to_owned)
                .collect()
        } else {
            return json!({"id":"old-downloads","ok":false,"freed":0,"error":crate::i18n::tr(self.ctx.lang(), "storage.trashFailed")});
        };
        let mut moved = 0;
        let mut size = 0;
        let mut errors = 0;
        for p in chosen {
            let result = self.ctx.call("files.trash", &json!({"paths":[p]}));
            if result.is_ok_and(|v| v["failed"].as_array().is_none_or(Vec::is_empty)) {
                moved += 1;
                size += allowed[&p]["size"].as_u64().unwrap_or(0);
            } else {
                errors += 1
            }
        }
        json!({"id":"old-downloads","ok":errors==0||moved>0,"freed":0,"movedToTrash":size,"count":moved,"error":if errors==0{None}else{Some(crate::i18n::tr(self.ctx.lang(), "storage.someNotTrashed"))}})
    }
}
fn res(id: &str, freed: i128, errors: u64, lang: Lang) -> Value {
    let ok = errors == 0 || freed > 0;
    let mut value = json!({"id":id,"ok":ok,"freed":freed.max(0)as u64,"error":null});
    if errors > 0 {
        value["failed"] = json!(errors);
        if !ok {
            value["error"] = json!(crate::i18n::tr(lang, "storage.someNotDeleted"))
        }
    }
    value
}
pub(super) fn trash_size() -> Value {
    let mut size = 0;
    let mut count = 0;
    let deadline = Some(now() + 5.);
    for path in crate::files::storage_trash_dirs() {
        let files = path.join("files");
        size += du(&files, deadline, true).0;
        count += fs::read_dir(&files)
            .map(|e| e.filter_map(Result::ok).count())
            .unwrap_or(0);
        size += du(&path.join("info"), deadline, true).0;
    }
    json!({"size":size,"count":count})
}
fn thumbnail_dir() -> PathBuf {
    std::env::var_os("WIN11SHELL_THUMBNAIL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join(".cache"))
                .join("thumbnails")
        })
}
fn download_dir() -> PathBuf {
    std::env::var_os("WIN11SHELL_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_dir("DOWNLOAD"))
}
fn old_downloads() -> Vec<Value> {
    let mut out = vec![];
    let cutoff = wall() - 90. * 86400.;
    let Ok(entries) = fs::read_dir(download_dir()) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let Ok(s) = fs::symlink_metadata(&path) else {
            continue;
        };
        if name.starts_with('.') || s.file_type().is_symlink() {
            continue;
        }
        let modified = s.mtime() as f64 + s.mtime_nsec() as f64 / 1e9;
        let accessed = s.atime() as f64 + s.atime_nsec() as f64 / 1e9;
        if modified.max(accessed) >= cutoff {
            continue;
        }
        let size = if s.is_dir() {
            du(&path, Some(now() + 2.), true).0
        } else {
            s.blocks() * 512
        };
        out.push(json!({"path":path,"name":name,"size":size,"modified":(modified*1000.)as i64,"accessed":(accessed*1000.)as i64,"isDir":s.is_dir()}));
    }
    out.sort_by_key(|o| std::cmp::Reverse(o["size"].as_u64().unwrap_or(0)));
    out
}
fn snap_disabled() -> Vec<Value> {
    if !command_exists("snap") {
        return vec![];
    }
    let mut env = std::env::vars().collect::<BTreeMap<_, _>>();
    env.insert("LC_ALL".into(), "C".into());
    let r = run_env("snap", &["list", "--all"], 10., Some(&env));
    if !r.ok {
        return vec![];
    }
    let mut out = vec![];
    for l in r.out.lines().skip(1) {
        let p = l.split_whitespace().collect::<Vec<_>>();
        if p.len() < 6 || !p[p.len() - 1].split(',').any(|s| s == "disabled") {
            continue;
        }
        let (name, revision) = (p[0], p[2]);
        if !snap_name(name) || !snap_revision(revision) {
            continue;
        }
        let path = format!("/var/lib/snapd/snaps/{name}_{revision}.snap");
        let size = fs::metadata(path).map(|s| s.blocks() * 512).unwrap_or(0);
        out.push(json!({"name":name,"revision":revision,"size":size}));
    }
    out
}
fn snap_name(s: &str) -> bool {
    !s.is_empty()
        && (s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn snap_revision(s: &str) -> bool {
    let s = s.strip_prefix('x').unwrap_or(s);
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
fn journal_estimate() -> (Option<u64>, Option<u64>) {
    let cutoff = wall() - 14. * 86400.;
    let (mut estimated, mut total, mut readable) = (0, 0, false);
    for base in ["/var/log/journal", "/run/log/journal"] {
        let Ok(dirs) = fs::read_dir(base) else {
            continue;
        };
        for dir in dirs.flatten() {
            if !fs::symlink_metadata(dir.path()).is_ok_and(|s| s.is_dir()) {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir.path()) else {
                continue;
            };
            for e in entries.flatten() {
                readable = true;
                let Ok(s) = fs::symlink_metadata(e.path()) else {
                    continue;
                };
                let size = s.blocks() * 512;
                total += size;
                let name = e.file_name().to_string_lossy().into_owned();
                if (name.contains('@') || name.ends_with('~')) && (s.mtime() as f64) < cutoff {
                    estimated += size;
                }
            }
        }
    }
    if !readable {
        let mut env = std::env::vars().collect::<BTreeMap<_, _>>();
        env.insert("LC_ALL".into(), "C".into());
        let r = run_env("journalctl", &["--disk-usage"], 5., Some(&env));
        if let Some((_, s)) = r.out.split_once("take up ") {
            let digits = s
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>();
            if let Some(n) = num(&digits) {
                let unit = s.chars().nth(digits.len()).unwrap_or(' ');
                let scale = match unit {
                    'K' => 1024f64,
                    'M' => 1024f64.powi(2),
                    'G' => 1024f64.powi(3),
                    'T' => 1024f64.powi(4),
                    _ => 1.,
                };
                return (None, Some((n * scale) as u64));
            }
        }
        return (None, None);
    }
    (Some(estimated), Some(total))
}
fn apt_size() -> u64 {
    let mut total = 0;
    for d in ["/var/cache/apt/archives", "/var/cache/apt/archives/partial"] {
        if let Ok(entries) = fs::read_dir(d) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().ends_with(".deb") {
                    total += fs::symlink_metadata(e.path())
                        .map(|s| s.blocks() * 512)
                        .unwrap_or(0);
                }
            }
        }
    }
    total
}
fn crash_files() -> Vec<PathBuf> {
    let uid = unsafe { libc::getuid() };
    fs::read_dir("/var/crash")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| fs::symlink_metadata(e.path()).is_ok_and(|s| s.uid() == uid && s.is_file()))
        .map(|e| e.path())
        .collect()
}
fn safe_cache_paths(home: &Path) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    for record in data()["_SAFE_CACHES"].as_array().unwrap() {
        let path = home.join(record[0].as_str().unwrap());
        if path.is_dir() && !path.is_symlink() {
            out.push((path, record[1].as_str().unwrap().into()));
        }
    }
    if let Ok(entries) = fs::read_dir(home.join(".cache")) {
        for e in entries.flatten() {
            if e.file_name()
                .to_string_lossy()
                .starts_with("qtshadercache-")
                && e.path().is_dir()
                && !e.path().is_symlink()
            {
                out.push((e.path(), "Qt-shaders".into()));
            }
        }
    }
    out
}
fn browser_paths(home: &Path, lang: &str) -> Vec<(String, String, PathBuf)> {
    let mut out = vec![];
    for (key, rels) in data()["_BROWSER_CACHES"].as_object().unwrap() {
        for rel in rels.as_array().unwrap() {
            let path = home.join(rel.as_str().unwrap());
            if path.is_dir() && !path.is_symlink() {
                out.push((key.clone(), tr(lang, &format!("b_{key}"), &[]), path));
            }
        }
    }
    out
}
fn browser_running() -> HashSet<String> {
    let uid = unsafe { libc::getuid() };
    let mut found = HashSet::new();
    for pid in names("/proc") {
        if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if !fs::metadata(format!("/proc/{pid}")).is_ok_and(|s| s.uid() == uid) {
            continue;
        }
        let Ok(exe) = fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        let low = exe.to_string_lossy().to_lowercase();
        let base = basename(&low);
        let key = if ["firefox", "firefox-bin", "firefox-esr"].contains(&base)
            || low.contains("/firefox/")
        {
            Some("firefox")
        } else if ["zen", "zen-bin"].contains(&base) || low.contains("/zen/") {
            Some("zen")
        } else if low.contains("/opt/google/chrome") {
            Some("chrome")
        } else if low.contains("chromium") {
            Some("chromium")
        } else if low.contains("brave") {
            Some("brave")
        } else if low.contains("vivaldi") {
            Some("vivaldi")
        } else if low.contains("msedge") || low.contains("microsoft/edge") {
            Some("edge")
        } else if low.contains("librewolf") {
            Some("librewolf")
        } else if base == "opera" || low.contains("/opera/") {
            Some("opera")
        } else {
            None
        };
        if let Some(k) = key {
            found.insert(k.into());
        }
    }
    found
}
fn clear_contents(path: &Path, keep_dirs: bool) -> Vec<PathBuf> {
    if !fs::symlink_metadata(path).is_ok_and(|s| s.is_dir()) {
        return vec![];
    }
    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return vec![path.to_owned()],
    };
    let mut errors = vec![];
    for e in entries.flatten() {
        let p = e.path();
        let result = match fs::symlink_metadata(&p) {
            Ok(s) if s.is_dir() => {
                if keep_dirs {
                    errors.extend(clear_contents(&p, false));
                    Ok(())
                } else {
                    fs::remove_dir_all(&p)
                }
            }
            Ok(_) => fs::remove_file(&p),
            Err(e) => Err(e),
        };
        if result.is_err() {
            errors.push(p)
        }
    }
    errors
}
const ADMIN_SCRIPT: &str = "for step in \"$@\"; do\n  case \"$step\" in\n    apt) if apt-get clean; then echo \"@@ok apt\"; else echo \"@@fail apt\"; fi ;;\n    journal) if journalctl --vacuum-time=14d >/dev/null 2>&1; then echo \"@@ok journal\"; else echo \"@@fail journal\"; fi ;;\n    snap:*) s=\"${step#snap:}\"; n=\"${s%%=*}\"; r=\"${s#*=}\";\n      if snap remove \"$n\" --revision=\"$r\" >/dev/null 2>&1; then echo \"@@ok snap\"; else echo \"@@fail snap\"; fi ;;\n  esac\ndone\n";
fn admin_helper() -> Option<PathBuf> {
    let p = std::env::var_os("WIN11SHELL_ADMIN_HELPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/libexec/win11shell/cleanup-helper"));
    let s = fs::symlink_metadata(&p).ok()?;
    let parent = fs::metadata(p.parent()?).ok()?;
    if s.is_file()
        && s.uid() == 0
        && s.mode() & 0o022 == 0
        && parent.uid() == 0
        && parent.mode() & 0o022 == 0
    {
        Some(p)
    } else {
        None
    }
}
fn run_admin(
    steps: &[String],
    ids: &[String],
    before: &BTreeMap<String, Option<u64>>,
    lang: Lang,
) -> Vec<Value> {
    let failed = |message: &str| {
        ids.iter()
            .map(|id| json!({"id":id,"ok":false,"freed":0,"error":message}))
            .collect::<Vec<_>>()
    };
    if !command_exists("pkexec") {
        return failed(&crate::i18n::tr(lang, "storage.noAdmin"));
    }
    if steps.iter().any(|s| {
        s != "apt"
            && s != "journal"
            && !s
                .strip_prefix("snap:")
                .and_then(|s| s.split_once('='))
                .is_some_and(|(n, r)| snap_name(n) && snap_revision(r))
    }) {
        return failed(&crate::i18n::tr(lang, "storage.badCommand"));
    }
    let env = std::env::vars()
        .filter(|(k, _)| {
            [
                "DISPLAY",
                "WAYLAND_DISPLAY",
                "XAUTHORITY",
                "PATH",
                "DBUS_SESSION_BUS_ADDRESS",
                "XDG_RUNTIME_DIR",
                "LANG",
            ]
            .contains(&k.as_str())
        })
        .collect::<BTreeMap<_, _>>();
    let mut command = if let Some(helper) = admin_helper() {
        vec![helper.to_string_lossy().into_owned()]
    } else {
        vec![
            "/bin/sh".into(),
            "-c".into(),
            ADMIN_SCRIPT.into(),
            "win11shell-cleanup".into(),
        ]
    };
    command.extend_from_slice(steps);
    let refs = command.iter().map(String::as_str).collect::<Vec<_>>();
    let r = run_env("pkexec", &refs, 900., Some(&env));
    if (r.rc == 126 || r.rc == 127) && !r.out.contains("@@") {
        return failed(&if r.rc == 126 {
            crate::i18n::tr(lang, "storage.adminCancelled")
        } else {
            crate::i18n::tr(lang, "storage.adminDenied")
        });
    }
    ids.iter()
        .map(|id| {
            let tag = match id.as_str() {
                "apt-cache" => "apt",
                "journal" => "journal",
                _ => "snap",
            };
            let ok =
                r.out.contains(&format!("@@ok {tag}")) && !r.out.contains(&format!("@@fail {tag}"));
            let after = match id.as_str() {
                "apt-cache" => Some(apt_size()),
                "old-snaps" => Some(
                    snap_disabled()
                        .iter()
                        .map(|s| s["size"].as_u64().unwrap_or(0))
                        .sum(),
                ),
                _ => journal_estimate().1,
            };
            let freed = match (before.get(id).copied().flatten(), after) {
                (Some(b), Some(a)) => b.saturating_sub(a),
                _ => 0,
            };
            json!({"id":id,"ok":ok,"freed":freed,"error":if ok{None}else{Some(crate::i18n::tr(lang, "storage.failed"))}})
        })
        .collect()
}
