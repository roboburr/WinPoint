//! sound.alerts.get / sound.alerts.set: GNOME Settings' alert sound (cc-alert-chooser-page.c of GNOME Settings 50)
//! and Ubuntu's startup sound, for Windows' "Meer geluidsinstellingen" › Geluiden (Geluidsschema, Opstartgeluid
//! afspelen; amendment 3).
//!
//! * Geen geluiden: `event-sounds` false.
//! * Standaard: `event-sounds` true and `theme-name` reset (`g_settings_reset`, as GNOME does), so it follows the
//!   schema's default (Ubuntu: Yaru) and reports `isDefault`.
//! * Klik, Snaar, Schommel, Brom: GNOME's custom theme `$XDG_DATA_HOME/sounds/__custom`: index.theme
//!   (`[Sound Theme]` Name, Inherits = the default theme, Directories = .), the symbolic links bell-terminal.ogg and
//!   bell-window-system.ogg to /usr/share/sounds/gnome/default/alerts/<name>.ogg, the directory's mtime renewed (the
//!   sound theme caches look at it), then `event-sounds` true and `theme-name` "__custom".
//! * Opstartgeluid afspelen: `org.gnome.shell.ubuntu startup-sound` "warty-startup" or "" (GNOME Settings' own
//!   mapping on Ubuntu).
//!
//! The keys go through `gsettings::write_values` and `reset_values` (allow/system.rs: event-sounds and theme-name
//! are this method's rows, startup-sound is a Write row). Every write and reset is checked before a file is touched;
//! a dry run touches nothing.
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::gsettings;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) const METHOD: &str = "sound.alerts.set";
pub(crate) const SCHEMA: &str = "org.gnome.desktop.sound";
pub(crate) const UBUNTU: &str = "org.gnome.shell.ubuntu";
pub(crate) const CUSTOM: &str = "__custom";
pub(crate) const STARTUP: &str = "warty-startup";
/// GNOME's alert sounds (the files of /usr/share/sounds/gnome/default/alerts).
pub(crate) const SOUNDS: &[&str] = &["click", "string", "swing", "hum"];
/// What the chooser offers, in its order.
pub(crate) const CHOICES: &[&str] = &["none", "default", "click", "string", "swing", "hum"];

/// Where the custom theme lives and where GNOME's alert files are (tests: temporary directories).
#[derive(Clone, Debug)]
pub(crate) struct Dirs {
    pub data: PathBuf,
    pub alerts: PathBuf,
}

impl Dirs {
    pub(crate) fn system() -> Dirs {
        Dirs {
            data: glib::user_data_dir(),
            alerts: PathBuf::from("/usr/share/sounds/gnome/default/alerts"),
        }
    }

    pub(crate) fn theme(&self) -> PathBuf {
        self.data.join("sounds").join(CUSTOM)
    }

    pub(crate) fn sound(&self, name: &str) -> PathBuf {
        self.alerts.join(format!("{name}.ogg"))
    }
}

/// The alert of the custom theme: the target of its bell-terminal.ogg link without ".ogg" (get_alert_name).
pub(crate) fn custom_alert(dirs: &Dirs) -> Option<String> {
    let target = std::fs::read_link(dirs.theme().join("bell-terminal.ogg")).ok()?;
    let base = target.file_name()?.to_string_lossy().into_owned();
    Some(base.strip_suffix(".ogg").unwrap_or(&base).to_owned())
}

/// The chooser's value for these keys: none, default, or one of [`SOUNDS`].
pub(crate) fn alert_of(event_sounds: bool, theme: &str, custom: Option<&str>) -> &'static str {
    if !event_sounds {
        return "none";
    }
    if theme != CUSTOM {
        return "default";
    }
    custom
        .and_then(|c| SOUNDS.iter().find(|s| **s == c).copied())
        .unwrap_or("default")
}

/// The GSettings keys behind the alerts: GNOME's (the default backend) in the service, a memory backend in tests.
pub(crate) trait Keys {
    fn read(&mut self, schema: &str, key: &str, lang: Lang) -> Value;
    fn write(
        &mut self,
        schema: &str,
        values: &[(&str, Value)],
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError>;
    /// `g_settings_reset` of one key (GNOME Settings' Standaard resets theme-name).
    fn reset(
        &mut self,
        schema: &str,
        key: &str,
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError>;
}

/// The default backend, through `gsettings::read_item` / `write_values` (this method's allow rows).
pub(crate) struct SystemKeys;

impl Keys for SystemKeys {
    fn read(&mut self, schema: &str, key: &str, lang: Lang) -> Value {
        gsettings::read_item(schema, None, key, lang).unwrap_or(Value::Null)
    }

    fn write(
        &mut self,
        schema: &str,
        values: &[(&str, Value)],
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError> {
        gsettings::write_values(METHOD, schema, None, values, lang, dry_run)
    }

    fn reset(
        &mut self,
        schema: &str,
        key: &str,
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError> {
        gsettings::reset_values(METHOD, schema, None, &[key], lang, dry_run)
    }
}

/// sound.alerts.get.
pub(crate) fn get(dirs: &Dirs, keys: &mut dyn Keys, lang: Lang) -> Value {
    let (events, theme, startup) = (
        keys.read(SCHEMA, "event-sounds", lang),
        keys.read(SCHEMA, "theme-name", lang),
        keys.read(UBUNTU, "startup-sound", lang),
    );
    let on = events["value"] != false;
    let name = theme["value"].as_str().unwrap_or("").to_owned();
    let custom = custom_alert(dirs);
    let startup_available = startup["available"] == true && startup["allowed"] == true;
    json!({
        "alert": alert_of(on, &name, custom.as_deref()),
        "choices": CHOICES,
        "available": SOUNDS.iter().filter(|s| dirs.sound(s).exists()).collect::<Vec<_>>(),
        "eventSounds": on,
        "theme": name,
        "defaultTheme": theme["default"],
        "writable": events["writable"] == true && theme["writable"] == true,
        "startupSound": startup_available && startup["value"].as_str().is_some_and(|s| !s.is_empty()),
        "startupAvailable": startup_available,
        "startupWritable": startup["writable"] == true,
    })
}

/// The writes of one sound.alerts.set call, checked by `plan` and done by `apply`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Plan {
    pub sound: Vec<(&'static str, Value)>,
    /// keys of [`SCHEMA`] reset after `sound` is written (Standaard: theme-name)
    pub reset: Vec<&'static str>,
    pub startup: Option<Value>,
    /// the alert whose custom theme is written first
    pub custom: Option<&'static str>,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)
}

/// The writes `args` ask for ({alert?, theme?, eventSounds?, startupSound?}; at least one).
pub(crate) fn plan(args: &Value, keys: &mut dyn Keys, lang: Lang) -> Result<Plan, ApiError> {
    let mut p = Plan::default();
    if let Some(a) = args.get("alert").filter(|v| !v.is_null()) {
        let a = a
            .as_str()
            .and_then(|a| CHOICES.iter().find(|c| **c == a).copied())
            .ok_or_else(|| bad(lang))?;
        match a {
            "none" => p.sound.push(("event-sounds", json!(false))),
            "default" => {
                p.sound.push(("event-sounds", json!(true)));
                p.reset.push("theme-name");
            }
            sound => {
                p.custom = Some(sound);
                p.sound.push(("event-sounds", json!(true)));
                p.sound.push(("theme-name", json!(CUSTOM)));
            }
        }
    }
    if let Some(t) = args.get("theme").filter(|v| !v.is_null()) {
        let t = t
            .as_str()
            .filter(|t| theme_name_ok(t))
            .ok_or_else(|| bad(lang))?;
        if p.sound.iter().any(|(k, _)| *k == "theme-name") || p.reset.contains(&"theme-name") {
            return Err(bad(lang));
        }
        p.sound.push(("theme-name", json!(t)));
    }
    if let Some(e) = args.get("eventSounds").filter(|v| !v.is_null()) {
        let e = e.as_bool().ok_or_else(|| bad(lang))?;
        if p.sound.iter().any(|(k, _)| *k == "event-sounds") {
            return Err(bad(lang));
        }
        p.sound.push(("event-sounds", json!(e)));
    }
    if let Some(s) = args.get("startupSound").filter(|v| !v.is_null()) {
        p.startup = Some(json!(if s.as_bool().ok_or_else(|| bad(lang))? {
            STARTUP
        } else {
            ""
        }));
    }
    if p.sound.is_empty() && p.startup.is_none() {
        return Err(bad(lang));
    }
    Ok(p)
}

/// An XDG sound theme name: a directory name, never a path.
fn theme_name_ok(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 128
        && t != "."
        && t != ".."
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// The custom theme as GNOME Settings writes it (set_custom_theme): index.theme, the two links, the mtime.
pub(crate) fn write_custom(dirs: &Dirs, sound: &str, inherits: &str) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = dirs.theme();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let index = dir.join("index.theme");
    let file = glib::KeyFile::new();
    if index.exists() {
        let _ = file.load_from_file(&index, glib::KeyFileFlags::KEEP_COMMENTS);
    }
    file.set_string("Sound Theme", "Name", "Custom");
    if !inherits.is_empty() {
        file.set_string("Sound Theme", "Inherits", inherits);
    }
    file.set_string("Sound Theme", "Directories", ".");
    file.save_to_file(&index)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let target = dirs.sound(sound);
    for link in ["bell-terminal.ogg", "bell-window-system.ogg"] {
        let path = dir.join(link);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::os::unix::fs::symlink(&target, &path)?;
    }
    touch_dir(&dir)
}

fn touch_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.set_modified(std::time::SystemTime::now())
}

/// sound.alerts.set on `dirs`: check every key, then the theme files, then the keys.
pub(crate) fn set(
    dirs: &Dirs,
    keys: &mut dyn Keys,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> ApiResult {
    let plan = plan(args, keys, lang)?;
    if let Some(sound) = plan.custom
        && !dirs.sound(sound).exists()
    {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "sound.alertMissing",
            NO_PARAMS,
        ));
    }
    let sound: Vec<(&str, Value)> = plan.sound.iter().map(|(k, v)| (*k, v.clone())).collect();
    if !sound.is_empty() {
        keys.write(SCHEMA, &sound, lang, true)?;
    }
    for key in &plan.reset {
        keys.reset(SCHEMA, key, lang, true)?;
    }
    if let Some(v) = &plan.startup {
        keys.write(UBUNTU, &[("startup-sound", v.clone())], lang, true)?;
    }
    if dry_run {
        let mut out = get(dirs, keys, lang);
        out["ok"] = json!(true);
        out["dryRun"] = json!(true);
        return Ok(out);
    }
    if let Some(sound) = plan.custom {
        let inherits = keys.read(SCHEMA, "theme-name", lang)["default"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        write_custom(dirs, sound, &inherits)
            .map_err(|_| ApiError::tr("failed", lang, "sound.alertFailed", NO_PARAMS))?;
    }
    if !sound.is_empty() {
        keys.write(SCHEMA, &sound, lang, false)?;
    }
    for key in &plan.reset {
        keys.reset(SCHEMA, key, lang, false)?;
    }
    if let Some(v) = &plan.startup {
        keys.write(UBUNTU, &[("startup-sound", v.clone())], lang, false)?;
    }
    let mut out = get(dirs, keys, lang);
    out["ok"] = json!(true);
    Ok(out)
}
