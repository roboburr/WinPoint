pub(crate) mod disks;
pub(crate) mod display;
pub(crate) mod index;
pub(crate) mod media;
pub(crate) mod notifications;
pub(crate) mod shell;
pub(crate) mod software;
pub(crate) mod storage;
pub(crate) mod sysmon;
pub(crate) mod system;
pub(crate) mod tray;
pub(crate) mod update;
pub(crate) mod winpoint;
// Remaining native services. Public wire contracts retain the original UI API.
use crate::{
    events::Events,
    files::{ApiError, ApiResult},
    settings::SettingsStore,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

type Dispatcher = dyn Fn(&str, &Value) -> ApiResult + Send + Sync;
pub(crate) struct ServiceContext {
    pub events: Arc<Events>,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub ui: PathBuf,
    pub kind: String,
    pub dry_run: bool,
    store: SettingsStore,
    caller: Mutex<Option<Arc<Dispatcher>>>,
    /// What the native GUI of this backend said about its windows (`update.setWatching`): WINDOWS_UNKNOWN until it
    /// says, then WINDOWS_SHOWN or WINDOWS_NONE ([`Self::watching`]).
    windows: AtomicU8,
}
const WINDOWS_UNKNOWN: u8 = 0;
const WINDOWS_SHOWN: u8 = 1;
const WINDOWS_NONE: u8 = 2;
impl ServiceContext {
    pub fn new(
        events: Arc<Events>,
        config_dir: PathBuf,
        cache_dir: PathBuf,
        ui: PathBuf,
        kind: String,
        dry_run: bool,
    ) -> Result<Arc<Self>, String> {
        let store = SettingsStore::new(config_dir.clone()).map_err(|e| e.message)?;
        Ok(Arc::new(Self {
            events,
            config_dir,
            cache_dir,
            ui,
            kind,
            dry_run,
            store,
            caller: Mutex::new(None),
            windows: AtomicU8::new(WINDOWS_UNKNOWN),
        }))
    }
    pub fn settings(&self) -> Value {
        self.store.snapshot().unwrap_or_else(|_| {
            serde_json::from_str(include_str!("../settings-defaults.json"))
                .expect("embedded settings")
        })
    }
    /// The language of this context's settings file (contracts C1). One `stat` per call: the parsed value is
    /// cached by the file's identity (`i18n::language_in`), so it is cheap inside loops too.
    pub fn lang(&self) -> crate::i18n::Lang {
        crate::i18n::language_in(&self.config_dir)
    }
    /// `"nl"` or `"en"`: [`Self::lang`] as the string the services pass around.
    pub fn language(&self) -> String {
        self.lang().code().into()
    }
    pub fn emit(&self, name: &str, data: Value) {
        self.events.emit(name, data);
    }
    pub fn subscriber_count(&self) -> usize {
        self.events.subscriber_count()
    }
    /// The native GUI of this backend (`--http-only`, its event stream open for as long as the process lives) says
    /// whether it shows a window (native/apps/settings/src/keepalive.rs: closing the window while an update job
    /// runs keeps the process).
    pub fn set_windows_shown(&self, shown: bool) {
        self.windows.store(if shown { WINDOWS_SHOWN } else { WINDOWS_NONE }, Ordering::SeqCst);
    }
    /// Whether someone sees what this host shows (update/pk.rs `notify`: no "installed" or "failed" notification
    /// then). A native GUI's own word once it has said it ([`Self::set_windows_shown`]): its event stream stays open
    /// without a window, so it is no sign of one. Until it has said so, an open event stream.
    pub fn watching(&self) -> bool {
        match self.windows.load(Ordering::SeqCst) {
            WINDOWS_SHOWN => true,
            WINDOWS_NONE => false,
            _ => self.subscriber_count() > 0,
        }
    }
    pub fn set_caller(&self, call: Arc<Dispatcher>) {
        *self.caller.lock().unwrap_or_else(|e| e.into_inner()) = Some(call);
    }
    pub fn call(&self, method: &str, args: &Value) -> ApiResult {
        let call = self
            .caller
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match call {
            Some(call) => call(method, args),
            None => Err(ApiError::tr("offline", self.lang(), "core.starting", crate::i18n::NO_PARAMS)),
        }
    }
}

// A host only starts the services its visible UI actually requests. In particular,
// a Desktop/Start event stream must not start Task Manager process sampling.
struct LazyService<T> {
    value: std::sync::OnceLock<Arc<T>>,
    initialize: Mutex<()>,
}
impl<T> Default for LazyService<T> {
    fn default() -> Self {
        Self {
            value: std::sync::OnceLock::new(),
            initialize: Mutex::new(()),
        }
    }
}
impl<T> LazyService<T> {
    fn load(
        &self,
        ctx: &Arc<ServiceContext>,
        create: impl FnOnce(Arc<ServiceContext>) -> Result<Arc<T>, String>,
    ) -> Result<Arc<T>, ApiError> {
        if let Some(value) = self.value.get() {
            return Ok(value.clone());
        }
        let _guard = self.initialize.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = self.value.get() {
            return Ok(value.clone());
        }
        let value = create(ctx.clone()).map_err(|message| ApiError::new("offline", &message))?;
        let _ = self.value.set(value.clone());
        Ok(value)
    }
}
fn invoke<T>(
    slot: &LazyService<T>,
    ctx: &Arc<ServiceContext>,
    create: impl FnOnce(Arc<ServiceContext>) -> Result<Arc<T>, String>,
    call: impl FnOnce(&Arc<T>) -> Option<ApiResult>,
) -> Option<ApiResult> {
    match slot.load(ctx, create) {
        Ok(service) => call(&service),
        Err(error) => Some(Err(error)),
    }
}
pub(crate) struct NativeServices {
    pub ctx: Arc<ServiceContext>,
    tray: LazyService<tray::Tray>,
    disks: LazyService<disks::Disks>,
    system: LazyService<system::System>,
    display: LazyService<display::Display>,
    media: LazyService<media::Media>,
    sysmon: LazyService<sysmon::Sysmon>,
    index: LazyService<index::Index>,
    storage: LazyService<storage::Storage>,
    software: LazyService<software::Software>,
    update: LazyService<update::Update>,
    // D-G74a: WinPoint's own update check (services/winpoint.rs)
    winpoint: LazyService<winpoint::WinPoint>,
    notifications: Arc<notifications::Notifications>,
}
impl NativeServices {
    pub fn new(
        ctx: Arc<ServiceContext>,
        notifications: Arc<notifications::Notifications>,
    ) -> Result<Arc<Self>, String> {
        let service = Arc::new(Self {
            ctx,
            tray: LazyService::default(),
            disks: LazyService::default(),
            system: LazyService::default(),
            display: LazyService::default(),
            media: LazyService::default(),
            sysmon: LazyService::default(),
            index: LazyService::default(),
            storage: LazyService::default(),
            software: LazyService::default(),
            update: LazyService::default(),
            winpoint: LazyService::default(),
            notifications,
        });
        // Explicit fixture switch for measured A/B comparison with the prior runtime.
        if std::env::var("WIN11_NATIVE_EAGER_SERVICES").as_deref() == Ok("1") {
            service
                .system
                .load(&service.ctx, system::System::new)
                .map_err(|e| e.message)?;
            service
                .display
                .load(&service.ctx, display::Display::new)
                .map_err(|e| e.message)?;
            service
                .media
                .load(&service.ctx, media::Media::new)
                .map_err(|e| e.message)?;
            service
                .sysmon
                .load(&service.ctx, sysmon::Sysmon::new)
                .map_err(|e| e.message)?;
            service
                .index
                .load(&service.ctx, index::Index::new)
                .map_err(|e| e.message)?;
            service
                .tray
                .load(&service.ctx, tray::Tray::new)
                .map_err(|e| e.message)?;
            service
                .disks
                .load(&service.ctx, disks::Disks::new)
                .map_err(|e| e.message)?;
            service
                .storage
                .load(&service.ctx, storage::Storage::new)
                .map_err(|e| e.message)?;
        }
        Ok(service)
    }
    pub fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if method.starts_with("software.") {
            return invoke(&self.software, &self.ctx, software::Software::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("update.") {
            return invoke(&self.update, &self.ctx, update::Update::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("winpoint.") {
            return invoke(&self.winpoint, &self.ctx, winpoint::WinPoint::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("notifications.") {
            return self.notifications.call(method, args);
        }
        if method.starts_with("tray.") {
            return invoke(&self.tray, &self.ctx, tray::Tray::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("disks.") {
            return invoke(&self.disks, &self.ctx, disks::Disks::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("storage.") {
            return invoke(&self.storage, &self.ctx, storage::Storage::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("system.") {
            return invoke(&self.system, &self.ctx, system::System::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("display.") {
            return invoke(&self.display, &self.ctx, display::Display::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("media.") {
            return invoke(&self.media, &self.ctx, media::Media::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("sysmon.") {
            return invoke(&self.sysmon, &self.ctx, sysmon::Sysmon::new, |s| {
                s.call(method, args)
            });
        }
        if method.starts_with("index.") {
            return invoke(&self.index, &self.ctx, index::Index::new, |s| {
                s.call(method, args)
            });
        }
        None
    }
    /// D-G74a: this host (win11-shell's backend, lib.rs run_http_only) owns WinPoint's update check: the service
    /// starts now, not on the first call, so its owner lock and daily schedule run from the start.
    pub fn winpoint_start(&self) {
        if let Ok(s) = self.winpoint.load(&self.ctx, winpoint::WinPoint::new) {
            s.start();
        }
    }
    pub fn shutdown(&self) {
        if let Some(s) = self.winpoint.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.update.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.software.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.tray.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.disks.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.storage.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.index.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.sysmon.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.media.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.display.value.get() {
            s.shutdown();
        }
        if let Some(s) = self.system.value.get() {
            s.shutdown();
        }
        self.notifications.shutdown();
    }
}
#[cfg(test)]
mod lazy_service_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn a_native_gui_says_whether_it_shows_a_window() {
        let root = std::env::temp_dir().join(format!("watching-{}", crate::events::nonce()));
        std::fs::create_dir_all(&root).unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = ServiceContext::new(events, root.join("config"), root.join("cache"), root.clone(), "settings".into(), true)
            .unwrap();
        assert!(!ctx.watching(), "no GUI word and no event stream");
        ctx.set_windows_shown(true);
        assert!(ctx.watching());
        ctx.set_windows_shown(false);
        assert!(!ctx.watching(), "the GUI's word, whatever its stream");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn expensive_services_start_once_on_demand_and_retry_failed_initialization() {
        let root = std::env::temp_dir().join(format!("lazy-service-{}", crate::events::nonce()));
        std::fs::create_dir_all(&root).unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = ServiceContext::new(
            events.clone(),
            root.join("config"),
            root.join("cache"),
            root.clone(),
            "desktop".into(),
            true,
        )
        .unwrap();
        let slot = Arc::new(LazyService::<usize>::default());
        let count = Arc::new(AtomicUsize::new(0));
        assert!(slot.value.get().is_none());
        assert!(
            slot.load(&ctx, |_| Err("fixture unavailable".into()))
                .is_err()
        );
        assert!(slot.value.get().is_none());
        let mut threads = Vec::new();
        for _ in 0..8 {
            let (slot, count, ctx) = (slot.clone(), count.clone(), ctx.clone());
            threads.push(std::thread::spawn(move || {
                *slot
                    .load(&ctx, |_| {
                        count.fetch_add(1, Ordering::SeqCst);
                        Ok(Arc::new(42))
                    })
                    .unwrap()
            }));
        }
        for thread in threads {
            assert_eq!(thread.join().unwrap(), 42);
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        events.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    // Task 7 of GE1b: ctx.language() inside per-item code (the sysmon process list, disks, storage) must not do a
    // full settings read. It is one stat plus a cached value; ctx.settings() (the 3b898e6 path) reads the file
    // under the settings lock.
    #[test]
    fn language_is_a_cached_stat_not_a_settings_read() {
        let root = std::env::temp_dir().join(format!("lang-cost-{}", crate::events::nonce()));
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let file = config.join("settings.json");
        let mut settings: Value = serde_json::from_str(include_str!("../settings-defaults.json")).unwrap();
        settings["language"] = "en".into();
        std::fs::write(&file, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = ServiceContext::new(events.clone(), config, root.join("cache"), root.clone(), "taskmgr".into(), true)
            .unwrap();
        assert_eq!(ctx.language(), "en");
        // set_modified moved the file's ctime to now: a parse within i18n.rs RACY (50 ms) of a change is not trusted
        // and the next call reads the file again. Past that window, one more call caches it for good (as i18n.rs's
        // own cache test does); without the wait, the loop below parsed the file until 50 ms had gone by.
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert_eq!(ctx.language(), "en");
        let parses = crate::i18n::language_parses();
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(ctx.lang(), crate::i18n::Lang::En);
        }
        let cached = started.elapsed();
        assert!(crate::i18n::language_parses() - parses < 1000, "the unchanged file was parsed again");
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(ctx.settings()["language"], "en");
        }
        let snapshot = started.elapsed();
        eprintln!(
            "ctx.lang(): {:?} per call; ctx.settings() (the 3b898e6 language()): {:?} per call",
            cached / 10_000,
            snapshot / 10_000
        );
        events.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }
}
