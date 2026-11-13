//! Native settings API, OS effects, and cross-app notifications.
#[path = "settings_effects.rs"]
mod effects;
use crate::{
    backend::Backend,
    events::Events,
    files::{ApiError, ApiResult},
    settings::{Change, SettingsStore},
    wallpaper,
};
use gtk::glib;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(crate) struct SettingsRuntime {
    store: SettingsStore,
    effects: effects::Effects,
    events: Arc<Events>,
    gate: Mutex<()>,
    stopped: AtomicBool,
    ui: PathBuf,
}
impl SettingsRuntime {
    pub(crate) fn new(
        _backend: &Backend,
        events: Arc<Events>,
        ui: &Path,
        kind: &str,
    ) -> Result<Option<Arc<Self>>, String> {
        if std::env::var_os("WIN11_REFERENCE_INSTANCE").is_some() {
            return Ok(None);
        }
        let config = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
        let runtime = Arc::new(Self {
            store: SettingsStore::new(config).map_err(|e| e.message)?,
            effects: effects::Effects {
                root: ui
                    .parent()
                    .unwrap_or(ui)
                    .canonicalize()
                    .unwrap_or_else(|_| ui.parent().unwrap_or(ui).to_owned()),
                config_home: glib::user_config_dir(),
                // These GNOME hosts replace the original --mode serve backend.
                layer: false,
                captions: kind == "panel",
                dry_run: crate::backend::dry_run(),
            },
            events: events.clone(),
            gate: Mutex::new(()),
            stopped: AtomicBool::new(false),
            ui: ui.canonicalize().unwrap_or_else(|_| ui.to_owned()),
        });
        if kind == "panel" {
            runtime.sync_wallpaper();
        }
        if let Ok(settings) = runtime.store.snapshot() {
            runtime.effects.captions_at_start(&settings);
        }
        let weak = Arc::downgrade(&runtime);
        events.set_settings_loader(Arc::new(move |invalidation| {
            let runtime = weak.upgrade()?;
            let _guard = runtime.gate.lock().unwrap_or_else(|e| e.into_inner());
            let mut change = runtime.store.reread_disk().ok().flatten()?;
            change.path = invalidation["path"].as_str().map(str::to_owned);
            runtime.sync_effects(&change);
            runtime.wallpaper_changed(&change);
            Some(payload(change))
        }));
        let weak = Arc::downgrade(&runtime);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(150));
                let Some(runtime) = weak.upgrade() else { break };
                if runtime.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let _guard = runtime.gate.lock().unwrap_or_else(|e| e.into_inner());
                if let Ok(Some(change)) = runtime.store.reload() {
                    runtime.sync_effects(&change);
                    runtime.wallpaper_changed(&change);
                    runtime.events.emit("settings.changed", payload(change));
                }
            }
        });
        Ok(Some(runtime))
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Relaxed) {
            return Some(Err(ApiError::tr(
                "offline",
                self.store.lang(),
                "settings.stopping",
                crate::i18n::NO_PARAMS,
            )));
        }
        let result = self.store.call(method, args)?;
        Some(result.map(|(value, change)| {
            if let Some(change) = change {
                let wallpaper_changed = relevant_wallpaper_change(&change);
                self.sync_effects(&change);
                // Another host may have committed in between our write and its notification.
                let change = self.store.reload().ok().flatten().unwrap_or(change);
                if wallpaper_changed || relevant_wallpaper_change(&change) {
                    self.sync_wallpaper();
                }
                self.events.emit("settings.changed", payload(change));
            }
            value
        }))
    }
    fn sync_effects(&self, change: &Change) {
        // The settings file lock spans reading the latest values and applying effects,
        // so an old cross-app event cannot overwrite a newer light/dark/family choice.
        if let Err(error) = self
            .store
            .with_snapshot(|settings| self.effects.apply(settings, &change.changed))
        {
            eprintln!("Systeeminstellingen lezen: {}", error.message);
        }
    }
    fn wallpaper_changed(&self, change: &Change) {
        if relevant_wallpaper_change(change) {
            self.sync_wallpaper();
        }
    }
    fn sync_wallpaper(&self) {
        if self.effects.dry_run {
            return;
        }
        let result = self
            .store
            .with_snapshot(|settings| wallpaper::sync(settings, &self.ui));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("GNOME-achtergrond bijwerken: {error}"),
            Err(error) => eprintln!("Achtergrondinstellingen lezen: {}", error.message),
        }
    }
    pub(crate) fn shutdown(&self) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::Relaxed);
    }
}
fn payload(change: Change) -> Value {
    json!({"settings":change.settings,"path":change.path,"changed":change.changed})
}
fn relevant_wallpaper_change(change: &Change) -> bool {
    change
        .changed
        .iter()
        .any(|path| wallpaper::is_relevant(path))
}
