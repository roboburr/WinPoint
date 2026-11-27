//! Native settings API, OS effects, and cross-app notifications.
#[path = "settings_effects.rs"]
pub(crate) mod effects;
use crate::{
    backend::Backend,
    events::Events,
    files::{ApiError, ApiResult},
    settings::{Change, SettingsStore},
    wallpaper,
};
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
    /// GNOME → settings.json (settings_mirror.rs), the panel host only.
    mirror: Mutex<Option<crate::settings_mirror::Watcher>>,
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
        let agreed = config.join("gnome-mirror.json");
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
                mirror: kind == "panel",
                dry_run: crate::backend::dry_run(),
            },
            events: events.clone(),
            gate: Mutex::new(()),
            stopped: AtomicBool::new(false),
            ui: ui.canonicalize().unwrap_or_else(|_| ui.to_owned()),
            mirror: Mutex::new(None),
        });
        if kind == "panel" {
            runtime.sync_wallpaper();
        }
        if let Ok(settings) = runtime.store.snapshot() {
            runtime.effects.captions_at_start(&settings);
        }
        crate::gnome_look::repair_native_snap_at_start(kind, runtime.effects.dry_run);
        if runtime.effects.mirror && !runtime.effects.dry_run {
            runtime.start_mirror(agreed);
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
                let change = merge_reload(change, self.store.reload().ok().flatten());
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
    /// The panel host's GNOME → settings.json mirror (settings_mirror.rs `watch`): at the start the side that moved
    /// since the values both last agreed on (`agreed`, next to settings.json) wins, and GNOME's changes patch
    /// settings.json through this runtime, so every host hears settings.changed.
    fn start_mirror(self: &Arc<Self>, agreed: PathBuf) {
        let weak = Arc::downgrade(self);
        let snapshot: crate::settings_mirror::Snapshot = Arc::new(move || {
            let runtime = weak.upgrade()?;
            if runtime.stopped.load(Ordering::Relaxed) {
                return None;
            }
            runtime.store.snapshot().ok()
        });
        let weak = Arc::downgrade(self);
        let patch: crate::settings_mirror::Patch = Arc::new(move |patch| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            if let Some(Err(error)) = runtime.call("settings.patch", &json!({ "patch": patch })) {
                eprintln!(
                    "settings mirror: GNOME values not taken over: {}",
                    error.message
                );
            }
        });
        let weak = Arc::downgrade(self);
        let locked: crate::settings_mirror::Locked = Arc::new(move |f: &mut dyn FnMut(&Value)| {
            let Some(runtime) = weak.upgrade() else {
                return false;
            };
            runtime.store.with_snapshot(|settings| f(settings)).is_ok()
        });
        let watcher = crate::settings_mirror::watch(snapshot, patch, locked, agreed);
        *self.mirror.lock().unwrap_or_else(|e| e.into_inner()) = Some(watcher);
    }
    pub(crate) fn shutdown(&self) {
        if let Some(watcher) = self.mirror.lock().unwrap_or_else(|e| e.into_inner()).take() {
            watcher.stop();
        }
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::Relaxed);
    }
}
/// Our write's change after a reload: the newer settings when another host committed in between, with the paths of
/// both writes (the shell renders per changed path, native/shell/src/core.rs `touches`, so a path left out is never
/// shown) and our write's label.
fn merge_reload(own: Change, newer: Option<Change>) -> Change {
    let Some(mut newer) = newer else { return own };
    for path in own.changed {
        if !newer.changed.contains(&path) {
            newer.changed.push(path);
        }
    }
    newer.path = own.path.or(newer.path);
    newer
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

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: Option<&str>, changed: &[&str], mode: &str) -> Change {
        Change {
            settings: json!({"theme": {"mode": mode}}),
            path: path.map(str::to_owned),
            changed: changed.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn a_peer_write_in_between_keeps_our_paths() {
        let own = change(Some("theme.mode"), &["theme.mode"], "dark");
        assert_eq!(merge_reload(own.clone(), None).changed, ["theme.mode"]);
        let peer = change(None, &["explorer.view"], "dark");
        let merged = merge_reload(own, Some(peer));
        assert_eq!(merged.changed, ["explorer.view", "theme.mode"]);
        assert_eq!(merged.path.as_deref(), Some("theme.mode"));
        assert_eq!(merged.settings["theme"]["mode"], "dark");
        let both = merge_reload(
            change(None, &["theme.mode"], "light"),
            Some(change(Some("x"), &["theme.mode"], "light")),
        );
        assert_eq!((both.changed.len(), both.path.as_deref()), (1, Some("x")));
    }
}
