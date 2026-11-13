use super::{ServiceContext, common::*};
use crate::files::ApiError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
struct State {
    data: Option<Value>,
    fresh: bool,
    running: bool,
    progress: Value,
}
pub(super) struct Categories {
    ctx: Arc<ServiceContext>,
    path: PathBuf,
    state: Mutex<State>,
}
impl Categories {
    pub fn new(ctx: Arc<ServiceContext>) -> Arc<Self> {
        let path = ctx.cache_dir.join("storage-categories.json");
        let data = fs::read(&path)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .filter(Value::is_object);
        Arc::new(Self {
            ctx,
            path,
            state: Mutex::new(State {
                data,
                fresh: false,
                running: false,
                progress: json!({"files":0,"bytes":0}),
            }),
        })
    }
    pub fn get(self: &Arc<Self>, refresh: bool) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let stale = state.data.is_none()
            || !state.fresh
            || refresh
            || state.data.as_ref().is_some_and(|d| {
                wall() * 1000. - d["scannedAt"].as_f64().unwrap_or(0.) > 15. * 60. * 1000.
            });
        if stale && !state.running {
            state.running = true;
            let weak = Arc::downgrade(self);
            std::thread::spawn(move || {
                let Some(c) = weak.upgrade() else { return };
                match c.compute(180.) {
                    Ok(data) => {
                        {
                            let mut state = c.state.lock().unwrap_or_else(|e| e.into_inner());
                            state.data = Some(data.clone());
                            state.fresh = true;
                        }
                        let _ = atomic(&c.path, data.to_string().as_bytes());
                        c.ctx.emit("storage.categoriesReady", json!({"data":data}));
                    }
                    Err(_) => {}
                }
                c.state.lock().unwrap_or_else(|e| e.into_inner()).running = false;
            });
        }
        let mut out = json!({"ready":state.data.is_some()&&!refresh,"data":state.data,"running":state.running});
        // The data is cached (15 minutes, and on disk) with its category names in the language of that scan: set
        // them again in the current language (round2-gate §2). Look the list up without inserting: `data` is null
        // before the first scan, and the UI shows its "Berekenen…" card only while it is (review B1).
        if let Some(categories) = out
            .get_mut("data")
            .and_then(|d| d.get_mut("categories"))
            .and_then(Value::as_array_mut)
        {
            let lang = self.ctx.language();
            for category in categories {
                if let Some(id) = category
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    category["name"] = json!(tr(&lang, &format!("c_{id}"), &[]));
                }
            }
        }
        if state.running {
            out["progress"] = state.progress.clone()
        }
        out
    }
    fn compute(&self, budget: f64) -> Result<Value, crate::files::ApiError> {
        scan_priority();
        let home = canonical(&home());
        let (roots, main) = roots(&home);
        let ids = data()["_CATEGORY_IDS"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut sizes = ids
            .iter()
            .map(|c| (c.clone(), 0u64))
            .collect::<HashMap<_, _>>();
        let mut items = HashMap::<(String, String), u64>::new();
        let mut item_order = Vec::new();
        let mut item_paths = HashMap::<(String, String), PathBuf>::new();
        let per_child = [
            "appdata",
            "cache",
            "flatpak",
            "snaps",
            "other",
            "documents",
            "downloads",
            "videos",
            "music",
            "pictures",
        ];
        let start = now();
        let deadline = start + budget;
        let hst = fs::metadata(&home).map_err(|_| {
            ApiError::tr(
                "not_found",
                self.ctx.lang(),
                "storage.noHome",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let dev = hst.dev();
        let mut seen = HashSet::new();
        let mut stack = vec![(
            home.clone(),
            0usize,
            "other".to_owned(),
            None::<String>,
            true,
        )];
        let mut files = 0u64;
        let mut total = hst.blocks() * 512;
        let mut partial = false;
        let mut last = now();
        let mut n = 0;
        while let Some((dir, depth, cat, item, split)) = stack.pop() {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for e in entries.flatten() {
                let full = e.path();
                let Ok(s) = fs::symlink_metadata(&full) else {
                    continue;
                };
                let size = s.blocks() * 512;
                if s.is_dir() {
                    if s.dev() != dev {
                        continue;
                    }
                    let root = roots.get(&full);
                    let (ccat, citem, csplit) = if let Some((ccat, citem)) = root {
                        (
                            ccat.clone(),
                            citem.clone(),
                            citem.is_none() && per_child.contains(&ccat.as_str()),
                        )
                    } else if e.file_name() == "node_modules" && depth < 4 {
                        ("dev".into(), Some("node_modules".into()), false)
                    } else if split {
                        let name = e.file_name().to_string_lossy().into_owned();
                        item_paths
                            .entry((cat.clone(), name.clone()))
                            .or_insert_with(|| full.clone());
                        (cat.clone(), Some(name), false)
                    } else {
                        (cat.clone(), item.clone(), false)
                    };
                    *sizes.get_mut(&ccat).unwrap() += size;
                    if let Some(item) = &citem {
                        add_item(
                            &mut items,
                            &mut item_order,
                            (ccat.clone(), item.clone()),
                            size,
                        );
                        if root.is_some() {
                            item_paths
                                .entry((ccat.clone(), item.clone()))
                                .or_insert_with(|| full.clone());
                        }
                    }
                    stack.push((full, depth + 1, ccat, citem, csplit));
                } else {
                    if s.nlink() > 1 && !seen.insert((s.dev(), s.ino())) {
                        continue;
                    }
                    *sizes.get_mut(&cat).unwrap() += size;
                    if let Some(item) = &item {
                        add_item(
                            &mut items,
                            &mut item_order,
                            (cat.clone(), item.clone()),
                            size,
                        );
                    }
                    files += 1;
                }
                total += size;
                n += 1;
                if n & 4095 == 0 {
                    let time = now();
                    if time > deadline {
                        partial = true;
                        stack.clear();
                        break;
                    }
                    if time - last > 1. {
                        last = time;
                        self.state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .progress = json!({"files":files,"bytes":total});
                        self.ctx.emit(
                            "storage.categoriesProgress",
                            json!({"files":files,"bytes":total}),
                        );
                    }
                    std::thread::yield_now();
                }
            }
        }
        let lang = self.ctx.language();
        let mut categories = vec![];
        for id in ids {
            let mut entry = json!({"id":id,"name":tr(&lang,&format!("c_{id}"),&[]),"size":sizes[&id],"path":main.get(&id).unwrap_or(&home)});
            let mut its = item_order
                .iter()
                .filter(|(c, _)| c == &id)
                .map(|key| (key.1.clone(), items[key]))
                .collect::<Vec<_>>();
            its.sort_by(|a, b| b.1.cmp(&a.1));
            if !its.is_empty() {
                entry["items"]=json!(its.iter().take(12).filter(|(_,size)|*size>0).map(|(name,size)|json!({"name":name,"size":size,"path":item_paths.get(&(id.clone(),name.clone()))})).collect::<Vec<_>>());
            }
            categories.push(entry);
        }
        categories.sort_by(|a, b| {
            (a["id"] == "other").cmp(&(b["id"] == "other")).then(
                b["size"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["size"].as_u64().unwrap_or(0)),
            )
        });
        Ok(
            json!({"home":home,"categories":categories,"total":total,"files":files,"partial":partial,"scannedAt":(wall()*1000.)as i64,"tookMs":((now()-start)*1000.)as i64}),
        )
    }
}
fn roots(
    home: &Path,
) -> (
    BTreeMap<PathBuf, (String, Option<String>)>,
    HashMap<String, PathBuf>,
) {
    let mut roots = BTreeMap::new();
    let mut main = HashMap::from([("other".into(), home.to_owned())]);
    let mut add = |rel: &str, cat: &str, item: Option<String>| {
        let p = if Path::new(rel).is_absolute() {
            PathBuf::from(rel)
        } else {
            home.join(rel)
        };
        let p = normalized(&p);
        if p == home {
            return;
        }
        roots.insert(p.clone(), (cat.into(), item));
        main.entry(cat.into()).or_insert(p);
    };
    for (cat, kind) in [
        ("documents", "DOCUMENTS"),
        ("pictures", "PICTURES"),
        ("videos", "VIDEOS"),
        ("music", "MUSIC"),
        ("downloads", "DOWNLOAD"),
        ("desktop", "DESKTOP"),
    ] {
        add(&user_dir(kind).to_string_lossy(), cat, None);
    }
    for (rel, cat) in [
        (".config", "appdata"),
        (".local/share", "appdata"),
        (".local/state", "appdata"),
        (".cache", "cache"),
        ("snap", "snaps"),
        (".var/app", "flatpak"),
        (".local/share/flatpak", "flatpak"),
    ] {
        add(rel, cat, None);
    }
    for (key, cat) in [
        ("_DEV_ROOTS", "dev"),
        ("_BROWSER_ROOTS", "browser"),
        ("_CHAT_ROOTS", "chat"),
    ] {
        for (rel, name) in data()[key].as_object().unwrap() {
            add(rel, cat, name.as_str().map(str::to_owned));
        }
    }
    add(".local/share/Trash", "trash", None);
    drop(add);
    main.insert("trash".into(), home.join(".local/share/Trash/files"));
    for cat in ["dev", "browser", "chat"] {
        main.insert(cat.into(), home.to_owned());
    }
    (roots, main)
}

fn add_item(
    map: &mut HashMap<(String, String), u64>,
    order: &mut Vec<(String, String)>,
    key: (String, String),
    size: u64,
) {
    if !map.contains_key(&key) {
        order.push(key.clone());
    }
    *map.entry(key).or_default() += size;
}
#[cfg(test)]
mod tests {
    use super::*;
    fn context(lang: &str, cache_file: Option<&str>) -> (PathBuf, Arc<ServiceContext>) {
        let root =
            std::env::temp_dir().join(format!("win11-storage-cat-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            json!({"language":lang}).to_string(),
        )
        .unwrap();
        if let Some(text) = cache_file {
            fs::create_dir_all(root.join("cache")).unwrap();
            fs::write(root.join("cache/storage-categories.json"), text).unwrap();
        }
        let ctx = ServiceContext::new(
            crate::events::Events::new(&root.join("events")).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "settings".into(),
            false,
        )
        .unwrap();
        (root, ctx)
    }
    // Review B1: before the first scan (no cache file, or one that is not an object) the reply keeps `data: null`
    // in both languages, which is what makes the UI show its "Berekenen…" card (3b898e6's reply shape).
    #[test]
    fn get_without_data_keeps_data_null() {
        for (lang, file) in [
            ("nl", None),
            ("en", None),
            ("nl", Some("[1,2]")),
            ("en", Some("{corrupt")),
        ] {
            let (root, ctx) = context(lang, file);
            let c = Categories::new(ctx.clone());
            // A scan is already running (the first call starts one), so get() starts no thread here.
            c.state.lock().unwrap().running = true;
            let out = c.get(false);
            assert_eq!(
                out,
                json!({"ready":false,"data":null,"running":true,"progress":{"files":0,"bytes":0}}),
                "{lang} {file:?}"
            );
            assert!(c.state.lock().unwrap().data.is_none());
            ctx.events.shutdown();
            let _ = fs::remove_dir_all(root);
        }
    }
    // Review B1: a cached object without a categories list is handed out as it is (no key inserted).
    #[test]
    fn get_keeps_a_cached_object_without_categories_as_it_is() {
        let scanned = (wall() * 1000.) as i64;
        let (root, ctx) = context("en", Some(&json!({"scannedAt":scanned}).to_string()));
        let c = Categories::new(ctx.clone());
        c.state.lock().unwrap().fresh = true;
        let out = c.get(false);
        assert_eq!(
            out,
            json!({"ready":true,"data":{"scannedAt":scanned},"running":false})
        );
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
    // R2-B: the cached categories (15 minutes, and on disk) are named in the language of the read
    #[test]
    fn cached_category_names_follow_the_language() {
        let (root, ctx) = context("en", None);
        let c = Categories::new(ctx.clone());
        {
            let mut state = c.state.lock().unwrap();
            state.data = Some(
                json!({"categories":[{"id":"documents","name":"Documenten"},{"id":"other","name":"Overig"}],
                                     "scannedAt":(wall()*1000.) as i64}),
            );
            state.fresh = true;
        }
        let out = c.get(false);
        assert_eq!(out["data"]["categories"][0]["name"], "Documents");
        assert_eq!(out["data"]["categories"][1]["name"], "Other");
        assert!(!c.state.lock().unwrap().running, "fresh data: no new scan");
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
