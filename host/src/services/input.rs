//! `input.*` (batch A): the xkeyboard-config catalogue, IBus engines, input sources, keyboard shortcuts and tablets
//! (notes/claude/settings-parity.md §5.2).
//!
//! Built by batch A's slices 1 and 2 (the pages Muis, Touchpad, Pen en tablet; Typen, Taalopties, Geavanceerde
//! toetsenbordinstellingen and Sneltoetsen). Slice 3's accessibility pages (Toegankelijkheid, Tekstgrootte, Visuele
//! effecten, Contrastthema's, Muisaanwijzer en aanraken, Tekstcursor) need no method of their own: they bind GNOME's
//! keys through `gsettings.*` (`input/a11y_tests.rs`), and the rendering pairs are host/src/settings_mirror/render.rs.
//! Slice 4's pages (Vergrootglas, Verteller, Audio, Toetsenbord, Muis of Toegankelijkheid) bind their keys the same
//! way; Verteller's "Instellingen voor Verteller openen" is the one method of its own (amendment 36).
//!
//! * `input.tablets {}` → `{tablets: [{vidpid, name, kind, buttons, rings, strips, dials, reversible, pad, styli}],
//!   touchscreens: [{vidpid, name}], monitors: [{connector, vendor, product, serial, name, builtin}]}`: the drawing
//!   tablets and pen displays as GNOME Settings' Wacom panel finds them (udev's database and /sys, the libwacom
//!   data files), the touchscreens (amendment 8) and the monitors a tablet or touchscreen can be mapped to (Mutter's
//!   DisplayConfig, `output` = [vendor, product, serial]). `input/tablets.rs`.
//! * `input.tablet.mapButtons {vidpid}`: GNOME Shell's pad OSD in edition mode for the tablet's pad, through the
//!   extension (inputBridge.js `input.padOsd`); the OSD writes the pad-button keys itself.
//! * `input.layouts {}`, `input.engines {}`: Taalopties' catalogue (xkeyboard-config, iso-codes, IBus components).
//!   `input/layouts.rs`.
//! * `input.sources.set {sources}` (dangerous; also the system keyboard through GNOME's pkexec helper),
//!   `input.options.set {lv3?, compose?}` (dangerous), `input.preview {layout}` (tecla), `input.engineSetup {id}`
//!   (GNOME Shell starts the engine's setup), `input.sources.current {}` / `input.sources.activate {index}` (GNOME
//!   Shell's input sources for the tray's input indicator, amendment 15). `input/sources.rs`.
//! * `input.capture {timeout?}`, `input.capture.cancel {}`: Sneltoets instellen's key, captured by GNOME Shell.
//!   `input/capture.rs`.
//! * `input.shortcuts {}`, `.set`, `.reset`, `.resetAll`, `.custom.add`, `.custom.update`, `.custom.remove` (the
//!   writes dangerous): GNOME Settings' Keyboard Shortcuts with WinPoint's two (amendment 13). `input/shortcuts.rs`.
//! * `input.screenReader.preferences {}`: Orca's own preferences window, through `ShowPreferences` of the running
//!   Orca on the session bus, as GNOME Settings' Screen Reader Configure (amendment 36). `input/screen_reader.rs`.
//!
//! Every write goes through `gsettings::write_values` with this service's method named, so the allow rows of
//! `gsettings/allow/input.rs` and B0's checks apply; a write under `ctx.dry_run` only validates. After a write the
//! event `input.changed {what}` (`sources`, `options`, `shortcuts`; `systemKeyboard` with `ok` when GNOME's helper
//! finished).
use super::ServiceContext;
use super::gsettings;
use crate::files::{ApiError, ApiResult};
use crate::i18n::NO_PARAMS;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod a11y_tests;
pub(crate) mod capture;
#[cfg(test)]
mod keyboard_tests;
pub(crate) mod layouts;
pub(crate) mod screen_reader;
pub(crate) mod shortcuts;
pub(crate) mod sources;
pub(crate) mod tablets;
#[cfg(test)]
mod tests;

/// The methods (name, dangerous), exactly as host/src/api-methods/input.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("input.tablets", false),
    ("input.tablet.mapButtons", false),
    ("input.layouts", false),
    ("input.engines", false),
    ("input.sources.set", true),
    ("input.options.set", true),
    ("input.preview", false),
    ("input.engineSetup", false),
    ("input.sources.current", false),
    ("input.sources.activate", false),
    ("input.capture", false),
    ("input.capture.cancel", false),
    ("input.shortcuts", false),
    ("input.shortcuts.set", true),
    ("input.shortcuts.reset", true),
    ("input.shortcuts.resetAll", true),
    ("input.shortcuts.custom.add", true),
    ("input.shortcuts.custom.update", true),
    ("input.shortcuts.custom.remove", true),
    ("input.screenReader.preferences", false),
];

/// A call into the GNOME Shell extension (crate::gnome::call_api; tests hand in a fake).
type Bridge<'a> = &'a dyn Fn(&str, &Value) -> Result<ApiResult, String>;

pub(crate) struct Input {
    ctx: Arc<ServiceContext>,
    /// The file system root of /run/udev, /sys, /usr/share/libwacom, the xkb rules, IBus's components and GNOME
    /// Settings' keybinding files ("/"; a fixture tree in tests).
    root: PathBuf,
    /// The GSettings backend of this service's reads and writes (GNOME's; a keyfile in tests).
    gs: gsettings::Config,
}

impl Input {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            root: PathBuf::from("/"),
            gs: gsettings::Config::default(),
        }))
    }

    fn env(&self) -> gsettings::Env {
        gsettings::Env::new(&self.gs, None)
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let bridge = |m: &str, a: &Value| crate::gnome::call_api(m, a);
        let timed = |m: &str, a: &Value, ms: i32| crate::gnome::call_api_with_timeout(m, a, ms);
        self.call_with(method, args, &bridge, &timed)
    }

    /// [`Input::call`] with the bridge handed in (tests: a fake extension).
    fn call_with(
        &self,
        method: &str,
        args: &Value,
        bridge: Bridge,
        timed: capture::TimedBridge,
    ) -> Option<ApiResult> {
        let (lang, dry_run, root) = (self.ctx.lang(), self.ctx.dry_run, self.root.as_path());
        Some(match method {
            "input.tablets" => Ok(self.tablets()),
            "input.tablet.mapButtons" => self.map_buttons(args, bridge),
            "input.layouts" => Ok(layouts::layouts(root, lang)),
            "input.engines" => Ok(layouts::engines(root, lang)),
            "input.sources.set" => {
                let (items, sync) = match sources::set_sources(&mut self.env(), args, lang, dry_run)
                {
                    Ok(done) => done,
                    Err(e) => return Some(Err(e)),
                };
                self.changed("sources", dry_run);
                let mut out = items;
                out["systemKeyboard"] = json!(if sync.is_some() {
                    "checking"
                } else {
                    "skipped"
                });
                if let Some(argv) = sync {
                    let events = self.ctx.events.clone();
                    sources::sync_system_keyboard(argv, move |ok| {
                        events.emit("input.changed", json!({"what": "systemKeyboard", "ok": ok}));
                    });
                }
                Ok(out)
            }
            "input.options.set" => self.written(
                "options",
                sources::set_options(&mut self.env(), args, lang, dry_run),
            ),
            "input.preview" => sources::preview(args, lang),
            "input.engineSetup" => sources::engine_setup(args, lang, bridge),
            "input.sources.current" => sources::current(lang, bridge),
            "input.sources.activate" => sources::activate(args, lang, bridge),
            "input.capture" => capture::capture(args, lang, timed),
            "input.capture.cancel" => capture::cancel(timed),
            "input.shortcuts" => Ok(shortcuts::state(&mut self.env(), root, lang).json()),
            "input.shortcuts.set" => {
                self.shortcut_write(shortcuts::set(&mut self.env(), root, args, lang, dry_run))
            }
            "input.shortcuts.reset" => self.written(
                "shortcuts",
                shortcuts::reset(&mut self.env(), root, args, lang, dry_run),
            ),
            "input.shortcuts.resetAll" => self.written(
                "shortcuts",
                shortcuts::reset_all(&mut self.env(), root, lang, dry_run),
            ),
            "input.shortcuts.custom.add" => self.shortcut_write(shortcuts::custom_add(
                &mut self.env(),
                root,
                args,
                lang,
                dry_run,
            )),
            "input.shortcuts.custom.update" => self.shortcut_write(shortcuts::custom_update(
                &mut self.env(),
                root,
                args,
                lang,
                dry_run,
            )),
            "input.shortcuts.custom.remove" => self.written(
                "shortcuts",
                shortcuts::custom_remove(&mut self.env(), root, args, lang, dry_run),
            ),
            "input.screenReader.preferences" => screen_reader::show_preferences(lang),
            _ => return None,
        })
    }

    /// `input.changed {what}` after a write that happened.
    fn changed(&self, what: &str, dry_run: bool) {
        if !dry_run {
            self.ctx.emit("input.changed", json!({ "what": what }));
        }
    }

    fn written(&self, what: &str, result: Result<Value, ApiError>) -> ApiResult {
        if result.is_ok() {
            self.changed(what, self.ctx.dry_run);
        }
        result
    }

    /// A shortcut write: no event for an answer that is a conflict (nothing was written).
    fn shortcut_write(&self, result: Result<Value, ApiError>) -> ApiResult {
        if result.as_ref().is_ok_and(|v| v["conflict"].is_null()) {
            self.changed("shortcuts", self.ctx.dry_run);
        }
        result
    }

    fn tablets(&self) -> Value {
        let mut value = tablets::scan(&self.root);
        value["monitors"] = json!(tablets::monitors());
        value
    }

    /// `input.tablet.mapButtons {vidpid}`: the pad OSD of a tablet that has a pad (buttons, rings, strips or dials).
    fn map_buttons(&self, args: &Value, bridge: Bridge) -> ApiResult {
        let lang = self.ctx.lang();
        let vidpid = args["vidpid"].as_str().unwrap_or("");
        if !tablets::vidpid_ok(vidpid) {
            return Err(ApiError::tr("bad_args", lang, "input.badTablet", NO_PARAMS));
        }
        let found = tablets::scan(&self.root);
        let tablet = found["tablets"]
            .as_array()
            .and_then(|list| list.iter().find(|t| t["vidpid"] == vidpid).cloned());
        let Some(tablet) = tablet else {
            return Err(ApiError::tr(
                "not_found",
                lang,
                "input.tabletNotFound",
                NO_PARAMS,
            ));
        };
        if tablet["pad"] != true {
            return Err(ApiError::tr("unsupported", lang, "input.noPad", NO_PARAMS));
        }
        match bridge("input.padOsd", &json!({ "vidpid": vidpid })) {
            Ok(result) => result,
            Err(_) => Err(ApiError::tr(
                "unsupported",
                lang,
                "input.shellUnavailable",
                NO_PARAMS,
            )),
        }
    }

    pub(crate) fn shutdown(&self) {}
}
