//! Native storage volumes, bounded directory scans, categories and original cleanup actions.
use super::ServiceContext;
use crate::{files::ApiResult, i18n::Lang};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::fd::AsRawFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
/// The methods (name, dangerous), as the api-methods files list them; batch D adds `storage.cleanTemp` here
/// (notes/claude/settings-parity.md §5.5; services/mod.rs `method_tables`).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("storage.categories", false),
    ("storage.cleanTemp", true),
    ("storage.cleanup.list", false),
    ("storage.cleanup.run", true),
    ("storage.largeFiles", false),
    ("storage.scan", false),
    ("storage.scanCancel", false),
    ("storage.scanResult", false),
    ("storage.trashSize", false),
    ("storage.volumes", false),
];
#[path = "storage/categories.rs"]
mod categories;
#[path = "storage/cleanup.rs"]
mod cleanup;
#[path = "storage/common.rs"]
pub(crate) mod common;
#[path = "storage/scan.rs"]
mod scan;
#[path = "storage/sense.rs"]
pub(crate) mod sense;
#[cfg(test)]
#[path = "storage/test_bus.rs"]
pub(crate) mod test_bus;
#[path = "storage/volumes.rs"]
mod volumes;
/// storage.volumes' 2 s cache. Its entries hold names in a language ("Systeemschijf" / "System drive"), so it is
/// kept per language: a call after a language switch builds the list again (round2-gate §2).
struct VolumeCache {
    time: f64,
    lang: Lang,
    visible: Option<Vec<Value>>,
    all: Option<Vec<Value>>,
}
impl VolumeCache {
    /// The cached list for `include` in `lang`, while it is younger than 2 s.
    fn get(&self, lang: Lang, include: bool, now: f64) -> Option<&Vec<Value>> {
        if now - self.time >= 2. || self.lang != lang {
            return None;
        }
        if include {
            self.all.as_ref()
        } else {
            self.visible.as_ref()
        }
    }
    fn put(&mut self, lang: Lang, include: bool, now: f64, values: Vec<Value>) {
        self.time = now;
        self.lang = lang;
        if include {
            self.all = Some(values);
            self.visible = None;
        } else {
            self.visible = Some(values);
            self.all = None;
        }
    }
}
pub(crate) struct Storage {
    ctx: Arc<ServiceContext>,
    scanner: Arc<scan::Scanner>,
    categories: Arc<categories::Categories>,
    cleanup: cleanup::Cleanup,
    volumes: Mutex<VolumeCache>,
    trash: Mutex<Option<(f64, Value)>>,
    stopped: AtomicBool,
}
impl Storage {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let service = Arc::new(Self {
            scanner: scan::Scanner::new(ctx.clone()),
            categories: categories::Categories::new(ctx.clone()),
            cleanup: cleanup::Cleanup::new(ctx.clone()),
            ctx,
            volumes: Mutex::new(VolumeCache {
                time: 0.,
                lang: Lang::Nl,
                visible: None,
                all: None,
            }),
            trash: Mutex::new(None),
            stopped: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            let Ok(mut file) = fs::File::open("/proc/self/mountinfo") else {
                return;
            };
            let mut poll = libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLERR | libc::POLLPRI,
                revents: 0,
            };
            let mut last = None;
            loop {
                if weak
                    .upgrade()
                    .is_none_or(|s| s.stopped.load(Ordering::Relaxed))
                {
                    break;
                }
                let events = unsafe { libc::poll(&mut poll, 1, 2000) };
                if events <= 0 {
                    continue;
                }
                let _ = file.seek(SeekFrom::Start(0));
                let mut data = Vec::new();
                let _ = file.read_to_end(&mut data);
                std::thread::sleep(Duration::from_millis(500));
                let Some(storage) = weak.upgrade() else { break };
                if storage.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let volumes = volumes::volumes(&storage.ctx.language(), false);
                let signature = volumes
                    .iter()
                    .map(|v| (v["id"].clone(), v["mount"].clone()))
                    .collect::<Vec<_>>();
                if last.as_ref() != Some(&signature) {
                    if last.is_some() {
                        storage
                            .ctx
                            .emit("storage.volumesChanged", json!({"volumes":volumes}));
                    }
                    last = Some(signature);
                }
            }
        });
        Ok(service)
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let result = match method {
            "storage.volumes" => {
                let include = common::truth(&args["all"]);
                let lang = self.ctx.lang();
                let mut cache = self.volumes.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(previous) = cache.get(lang, include, common::now()) {
                    Ok(json!(previous))
                } else {
                    let values = volumes::volumes(lang.code(), include);
                    cache.put(lang, include, common::now(), values.clone());
                    Ok(json!(values))
                }
            }
            "storage.scan" => self.scanner.start(
                &args["path"],
                &args["maxDepth"],
                &args["topN"],
                common::truth(&args["refresh"]),
            ),
            "storage.scanCancel" => Ok(self.scanner.cancel(&args["jobId"])),
            "storage.scanResult" => self.scanner.result(&args["jobId"]),
            "storage.categories" => Ok(self.categories.get(common::truth(&args["refresh"]))),
            "storage.cleanup.list" => Ok(json!(self.cleanup.list(common::truth(&args["refresh"])))),
            "storage.cleanup.run" if self.ctx.dry_run => {
                let ids = if let Some(a) = args["ids"].as_array() {
                    a.clone()
                } else {
                    vec![args["ids"].clone()]
                };
                let allowed = common::data()["_CLEANUP_IDS"].as_array().unwrap();
                Ok(
                    json!({"ok":true,"dryRun":true,"results":ids.iter().filter(|i|allowed.contains(i)).map(|i|json!({"id":i,"ok":true,"freed":0,"error":null})).collect::<Vec<_>>(),"freed":0}),
                )
            }
            "storage.cleanup.run" => {
                let result = self.cleanup.run(&args["ids"], &args["paths"]);
                *self.trash.lock().unwrap_or_else(|e| e.into_inner()) = None;
                result
            }
            "storage.cleanTemp" => {
                let bus = super::system::common::session().ok();
                let session = bus.as_ref().map(|bus| (bus, sense::HOUSEKEEPING));
                let result = sense::clean_temp(args, self.ctx.lang(), self.ctx.dry_run, session);
                if let Ok(done) = &result
                    && done["trash"]["removed"].as_u64().is_some_and(|n| n > 0)
                {
                    *self.trash.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    self.ctx.emit("files.trashChanged", json!({}));
                }
                result
            }
            "storage.largeFiles" => {
                let path = if common::truth(&args["path"]) {
                    args["path"].clone()
                } else {
                    json!(common::home())
                };
                scan::large(&self.scanner, &path, &args["limit"], &args["minSize"])
            }
            "storage.trashSize" => {
                let mut cache = self.trash.lock().unwrap_or_else(|e| e.into_inner());
                if !common::truth(&args["refresh"]) {
                    if let Some((at, value)) = &*cache {
                        if common::now() - at < 30. {
                            return Some(Ok(value.clone()));
                        }
                    }
                }
                let value = cleanup::trash_size();
                *cache = Some((common::now(), value.clone()));
                Ok(value)
            }
            _ => return None,
        };
        Some(result)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.scanner.shutdown();
    }
}
// R2-B: the storage stem in English through its call sites, and storage.volumes' cache kept per language
// (round2-gate §2).
#[cfg(test)]
mod i18n_tests {
    use super::*;
    use crate::files::ApiError;

    fn write_language(config: &std::path::Path, lang: &str) {
        let temp = config.join("settings.json.new");
        fs::write(&temp, json!({"language":lang}).to_string()).unwrap();
        fs::rename(&temp, config.join("settings.json")).unwrap();
    }
    fn context(root: &std::path::Path, lang: &str) -> Arc<ServiceContext> {
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        write_language(&config, lang);
        ServiceContext::new(
            crate::events::Events::new(&root.join("events")).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "settings".into(),
            false,
        )
        .unwrap()
    }
    fn error(result: Option<ApiResult>) -> ApiError {
        result.expect("a storage method").expect_err("an error")
    }

    #[test]
    fn storage_errors_render_in_english_and_dutch_through_the_service() {
        let root =
            std::env::temp_dir().join(format!("win11-storage-i18n-{}", crate::events::nonce()));
        let ctx = context(&root, "en");
        let svc = Storage::new(ctx.clone()).unwrap();
        for (lang, folder, scannable, gone, unknown, nothing) in [
            (
                "en",
                "Choose a folder.",
                "This folder can’t be scanned.",
                "This folder doesn’t exist.",
                "This scan is no longer available.",
                "Choose what you want to clean up.",
            ),
            (
                "nl",
                "Geef een map op.",
                "Deze map kan niet worden gescand.",
                "Deze map bestaat niet.",
                "Deze scan is niet (meer) bekend.",
                "Kies wat je wilt opruimen.",
            ),
        ] {
            write_language(&ctx.config_dir, lang);
            let e = error(svc.call("storage.scan", &json!({"path":""})));
            assert_eq!((e.code.as_str(), e.message.as_str()), ("bad_args", folder));
            assert_eq!(e.key.as_deref(), Some("storage.noFolder"));
            assert_eq!(
                error(svc.call("storage.scan", &json!({"path":"/proc"}))).message,
                scannable
            );
            let e = error(svc.call(
                "storage.largeFiles",
                &json!({"path":"/nonexistent-r2b-storage"}),
            ));
            assert_eq!(e.message, gone);
            assert_eq!(
                error(svc.call("storage.scanResult", &json!({"jobId":"none"}))).message,
                unknown
            );
            // refused before anything is cleaned
            let e = error(svc.call("storage.cleanup.run", &json!({"ids":[]})));
            assert_eq!(e.message, nothing);
        }
        svc.shutdown();
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_volume_cache_is_used_only_in_its_language() {
        let mut cache = VolumeCache {
            time: 0.,
            lang: Lang::Nl,
            visible: None,
            all: None,
        };
        let nl = vec![json!({"name":"Systeemschijf"})];
        cache.put(Lang::Nl, false, 100., nl.clone());
        assert_eq!(cache.get(Lang::Nl, false, 101.), Some(&nl));
        assert_eq!(
            cache.get(Lang::En, false, 101.),
            None,
            "a switch within 2 s rebuilds the list"
        );
        assert_eq!(
            cache.get(Lang::Nl, true, 101.),
            None,
            "the other list is not cached"
        );
        assert_eq!(cache.get(Lang::Nl, false, 102.5), None, "older than 2 s");
        let en = vec![json!({"name":"System drive"})];
        cache.put(Lang::En, false, 101., en.clone());
        assert_eq!(cache.get(Lang::En, false, 101.5), Some(&en));
        assert_eq!(cache.get(Lang::Nl, false, 101.5), None);
    }

    #[test]
    fn storage_volumes_follow_a_switch_within_the_cache_window() {
        let root =
            std::env::temp_dir().join(format!("win11-storage-vol-{}", crate::events::nonce()));
        let ctx = context(&root, "nl");
        let svc = Storage::new(ctx.clone()).unwrap();
        let names = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["isSystem"] == true)
                .filter_map(|v| v["name"].as_str().map(str::to_owned))
                .collect()
        };
        let nl = svc.call("storage.volumes", &json!({})).unwrap().unwrap();
        write_language(&ctx.config_dir, "en");
        let en = svc.call("storage.volumes", &json!({})).unwrap().unwrap();
        // The root file system is listed when it is a real file system (volumes.rs: _REAL_FS, no loop device),
        // as on this host (ext4). Only where it is not (an overlay container) is there no name to check
        // (review m2: the check is no longer skipped silently).
        let real = common::data()["_REAL_FS"].as_array().unwrap();
        let root_is_real = volumes::parse_mountinfo(
            &fs::read_to_string("/proc/self/mountinfo").unwrap_or_default(),
        )
        .iter()
        .any(|m| {
            m["mount"] == "/"
                && real.contains(&m["fstype"])
                && !m["source"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("/dev/loop"))
        });
        if root_is_real {
            assert_eq!(names(&nl), ["Systeemschijf"]);
            assert_eq!(
                names(&en),
                ["System drive"],
                "the 2 s cache must not keep the Dutch name"
            );
        } else {
            eprintln!("no real root file system here: storage.volumes lists no system drive");
            assert!(names(&nl).is_empty() && names(&en).is_empty());
        }
        svc.shutdown();
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
