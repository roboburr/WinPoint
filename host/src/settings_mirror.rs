//! The preferences WinPoint shares with GNOME (notes/claude/settings-parity.md §6.4, amendments 47-49). Settings
//! pages write the GNOME key (its allow row is `Mirrored(setting)`); WinPoint's other UIs (the DND flyout, Start, the
//! clock menu) keep writing settings.json. The mirror, in the panel host only, keeps both sides equal:
//!
//! * settings.json → GNOME: [`apply`], the hook of settings_effects.rs `Effects::apply` after every settings change;
//! * GNOME → settings.json: [`watch`], a GLib thread that follows the paired keys and patches settings.json.
//!
//! Both directions write only a value that differs (the equality guard), and the GNOME writes are tagged ([`ECHO`],
//! the origin of a change) so the watcher does not hand the mirror's own write back.
//!
//! The start, at every panel-host start: the value both sides last agreed on is kept per pair in
//! `<config>/gnome-mirror.json` ([`AGREED_FILE`]). A side that moved since then wins: GNOME's value (a change made in
//! a GNOME or Ubuntu session) goes to settings.json, a settings.json change (made by another host while the panel was
//! down) goes to GNOME; with no record yet (the first start, amendment 47) GNOME wins. The look pair (the accent) goes
//! from settings.json to GNOME.
//!
//! The preference pairs are shared with GNOME exactly as GNOME Settings writes them, with no gnome_look record. The
//! look pair (`theme.accent` → `accent-color`, the nearest of GNOME's ten names, while `theme.syncSystem`) goes through
//! `gnome_look::note`, so `win11-app --gnome-look` puts GNOME's accent back in GNOME and Ubuntu sessions. The three
//! rendering pairs (text scale, reduced motion, high contrast) are batch A's ([`render`]).
use gio::prelude::*;
use glib::variant::ToVariant;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) mod render;
#[cfg(test)]
mod tests;

const INTERFACE: &str = "org.gnome.desktop.interface";
const CALENDAR: &str = "org.gnome.desktop.calendar";
const ACCENT_KEY: &str = "accent-color";

/// How a WinPoint value maps to a GNOME value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conv {
    /// bool ↔ b
    Bool,
    /// bool ↔ !b (notifications.dnd ↔ show-banners)
    Inverted,
    /// time.h24 ↔ clock-format '24h' / '12h'
    ClockFormat,
    /// time.firstDayOfWeek -1 (the region's), 0 Sunday … 6 Saturday ↔ week-start-day 'default', 'sunday' …
    WeekStart,
    /// accessibility.textScale (percent) ↔ text-scaling-factor (batch A)
    #[allow(dead_code)]
    Percent,
    /// accessibility.reduceMotion ↔ reduced-motion 'reduce' / 'no-preference' (batch A)
    #[allow(dead_code)]
    ReducedMotion,
}

/// One pair: WinPoint's settings path(s) and GNOME's key. With two paths, GNOME's value goes to both and the one
/// that changed goes to GNOME.
#[derive(Debug)]
pub(crate) struct Pair {
    pub settings: &'static [&'static str],
    pub schema: &'static str,
    pub key: &'static str,
    pub conv: Conv,
}

/// The preference pairs (B0).
pub(crate) const PAIRS: &[Pair] = &[
    Pair {
        settings: &["notifications.dnd"],
        schema: "org.gnome.desktop.notifications",
        key: "show-banners",
        conv: Conv::Inverted,
    },
    Pair {
        settings: &["time.h24"],
        schema: INTERFACE,
        key: "clock-format",
        conv: Conv::ClockFormat,
    },
    Pair {
        settings: &["time.showSeconds", "taskbar.showSeconds"],
        schema: INTERFACE,
        key: "clock-show-seconds",
        conv: Conv::Bool,
    },
    Pair {
        settings: &["time.showDate"],
        schema: INTERFACE,
        key: "clock-show-date",
        conv: Conv::Bool,
    },
    Pair {
        settings: &["time.showWeekday"],
        schema: INTERFACE,
        key: "clock-show-weekday",
        conv: Conv::Bool,
    },
    Pair {
        settings: &["time.weekNumbers"],
        schema: CALENDAR,
        key: "show-weekdate",
        conv: Conv::Bool,
    },
    Pair {
        settings: &["time.firstDayOfWeek"],
        schema: CALENDAR,
        key: "week-start-day",
        conv: Conv::WeekStart,
    },
    Pair {
        settings: &["privacy.recentFiles"],
        schema: "org.gnome.desktop.privacy",
        key: "remember-recent-files",
        conv: Conv::Bool,
    },
    Pair {
        settings: &["taskbar.batteryPercent"],
        schema: INTERFACE,
        key: "show-battery-percentage",
        conv: Conv::Bool,
    },
    // Alt+Tab (WinPoint's Taakweergave) and GNOME Settings' App Switching agree also while Multitasking is closed
    // (batch D's review fix D-7); the window switcher's key stays a plain Write row that the page writes along
    Pair {
        settings: &["taskView.allDesktops"],
        schema: "org.gnome.shell.app-switcher",
        key: "current-workspace-only",
        conv: Conv::Inverted,
    },
];

/// Every pair: B0's and batch A's rendering pairs.
pub(crate) fn all_pairs() -> impl Iterator<Item = &'static Pair> {
    PAIRS.iter().chain(render::PAIRS.iter())
}

const DAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// WinPoint's value → GNOME's (None: no usable value).
pub(crate) fn to_gnome(conv: Conv, value: &Value) -> Option<glib::Variant> {
    Some(match conv {
        Conv::Bool => value.as_bool()?.to_variant(),
        Conv::Inverted => (!value.as_bool()?).to_variant(),
        Conv::ClockFormat => (if value.as_bool()? { "24h" } else { "12h" }).to_variant(),
        Conv::WeekStart => match value.as_i64()? {
            -1 => "default".to_variant(),
            n @ 0..=6 => DAYS[n as usize].to_variant(),
            _ => return None,
        },
        Conv::Percent => {
            let percent = value.as_f64().filter(|p| p.is_finite() && *p > 0.0)?;
            (percent / 100.0).to_variant()
        }
        Conv::ReducedMotion => (if value.as_bool()? {
            "reduce"
        } else {
            "no-preference"
        })
        .to_variant(),
    })
}

/// GNOME's value → WinPoint's (None: a value WinPoint cannot hold).
pub(crate) fn to_winpoint(conv: Conv, value: &glib::Variant) -> Option<Value> {
    Some(match conv {
        Conv::Bool => json!(value.get::<bool>()?),
        Conv::Inverted => json!(!value.get::<bool>()?),
        Conv::ClockFormat => match value.str()? {
            "24h" => json!(true),
            "12h" => json!(false),
            _ => return None,
        },
        Conv::WeekStart => match value.str()? {
            "default" => json!(-1),
            day => json!(DAYS.iter().position(|d| *d == day)?),
        },
        Conv::Percent => {
            // a whole percent is an integer, as settings.json holds it (GNOME's 1.0 is 100, the default); any
            // other value keeps its decimals (amendment 48: never round a GNOME value and write it back), without
            // the binary noise of the multiplication
            let percent = value.get::<f64>()? * 100.0;
            if !percent.is_finite() {
                return None;
            }
            if (percent - percent.round()).abs() < 1e-9 {
                json!(percent.round() as i64)
            } else {
                json!((percent * 1e9).round() / 1e9)
            }
        }
        Conv::ReducedMotion => json!(value.str()? == "reduce"),
    })
}

fn at<'a>(settings: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(settings, |v, k| &v[k])
}

/// The same WinPoint value: equal, or the same number (settings.json's 125 and 125.0).
fn same(a: &Value, b: &Value) -> bool {
    a == b || matches!((a.as_f64(), b.as_f64()), (Some(x), Some(y)) if x == y)
}

/// Whether GNOME's `current` and WinPoint's `value` agree, compared in WinPoint's domain (a GNOME value WinPoint
/// cannot hold never agrees): GNOME's 1.234 and settings.json's 123.4 agree, though 123.4 / 100 is not exactly 1.234.
fn agrees(conv: Conv, value: &Value, current: &glib::Variant) -> bool {
    to_winpoint(conv, current).is_some_and(|w| same(&w, value))
}

/// The setting of `pair` a change touches (a changed path equal to, above or below it), if any.
fn touched(pair: &Pair, changed: &[String]) -> Option<&'static str> {
    pair.settings.iter().copied().find(|s| {
        changed.iter().any(|c| {
            c == s
                || c.is_empty()
                || s.starts_with(&format!("{c}."))
                || c.starts_with(&format!("{s}."))
        })
    })
}

/// The GNOME side of the mirror: GNOME's default backend in the hosts, a memory backend in tests.
pub(crate) trait Gnome {
    fn read(&self, schema: &str, key: &str) -> Option<glib::Variant>;
    fn writable(&self, schema: &str, key: &str) -> bool;
    /// Whether the installed key takes `value` (its type and range; Ubuntu's accent-color has "brown", GNOME's not).
    fn accepts(&self, schema: &str, key: &str, value: &glib::Variant) -> bool;
    fn write(&self, schema: &str, key: &str, value: &glib::Variant) -> bool;
    /// A WinPoint look key: the checks of `write`, then `gnome_look::note` (only for a write that will happen), then
    /// the write.
    fn write_look(&self, schema: &str, key: &str, value: &glib::Variant) -> bool;
}

/// GNOME's settings objects, opened once per schema (on the calling thread).
pub(crate) struct Live {
    backend: Option<gio::SettingsBackend>,
    open: RefCell<HashMap<String, Option<gio::Settings>>>,
}

impl Live {
    pub(crate) fn new() -> Live {
        Live::with_backend(None)
    }

    pub(crate) fn with_backend(backend: Option<gio::SettingsBackend>) -> Live {
        Live {
            backend,
            open: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn settings(&self, schema: &str) -> Option<gio::Settings> {
        self.open
            .borrow_mut()
            .entry(schema.to_owned())
            .or_insert_with(|| {
                let source = gio::SettingsSchemaSource::default()?;
                let schema = source.lookup(schema, true)?;
                schema
                    .path()
                    .is_some()
                    .then(|| gio::Settings::new_full(&schema, self.backend.as_ref(), None))
            })
            .clone()
    }

    fn with_key(&self, schema: &str, key: &str) -> Option<gio::Settings> {
        self.settings(schema)
            .filter(|s| s.settings_schema().is_some_and(|sc| sc.has_key(key)))
    }
}

fn takes(settings: &gio::Settings, key: &str, value: &glib::Variant) -> bool {
    let Some(schema) = settings.settings_schema() else {
        return false;
    };
    let k = schema.key(key);
    value.type_() == k.value_type() && k.range_check(value)
}

impl Gnome for Live {
    fn read(&self, schema: &str, key: &str) -> Option<glib::Variant> {
        self.with_key(schema, key).map(|s| s.value(key))
    }

    fn writable(&self, schema: &str, key: &str) -> bool {
        self.with_key(schema, key)
            .is_some_and(|s| s.is_writable(key))
    }

    fn accepts(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        self.with_key(schema, key)
            .is_some_and(|s| takes(&s, key, value))
    }

    fn write(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        self.with_key(schema, key).is_some_and(|s| {
            takes(&s, key, value) && s.is_writable(key) && s.set_value(key, value).is_ok()
        })
    }

    fn write_look(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        let Some(settings) = self.with_key(schema, key) else {
            return false;
        };
        // the record only for a write that happens: a refused one would make WinPoint's value look like GNOME's
        if !takes(&settings, key, value) || !settings.is_writable(key) {
            return false;
        }
        crate::gnome_look::note(&settings, key, value);
        settings.set_value(key, value).is_ok()
    }
}

/// settings.json → GNOME: the writes a change needs, only where GNOME's value differs.
pub(crate) fn gnome_writes(
    settings: &Value,
    changed: &[String],
    gnome: &dyn Gnome,
) -> Vec<(&'static Pair, glib::Variant)> {
    writes_of(all_pairs(), settings, changed, gnome)
}

/// [`gnome_writes`] for `pairs` (tests: a pair of batch A's before it merges).
fn writes_of(
    pairs: impl Iterator<Item = &'static Pair>,
    settings: &Value,
    changed: &[String],
    gnome: &dyn Gnome,
) -> Vec<(&'static Pair, glib::Variant)> {
    pairs
        .filter_map(|pair| {
            let path = touched(pair, changed)?;
            let value = at(settings, path);
            let target = to_gnome(pair.conv, value)?;
            let current = gnome.read(pair.schema, pair.key)?;
            (!agrees(pair.conv, value, &current)).then_some((pair, target))
        })
        .collect()
}

fn set_path(patch: &mut Value, path: &str, value: Value) {
    let mut node = patch;
    let parts: Vec<&str> = path.split('.').collect();
    for part in &parts[..parts.len() - 1] {
        if !node[*part].is_object() {
            node[*part] = json!({});
        }
        node = &mut node[*part];
    }
    node[parts[parts.len() - 1]] = value;
}

/// GNOME → settings.json: the patch that brings the settings to GNOME's values (an empty object when nothing
/// differs); `only` limits it to one GNOME key.
pub(crate) fn winpoint_patch(
    settings: &Value,
    gnome: &dyn Gnome,
    only: Option<(&str, &str)>,
) -> Value {
    let mut patch = Value::Object(Map::new());
    for pair in all_pairs() {
        if only.is_some_and(|(schema, key)| schema != pair.schema || key != pair.key) {
            continue;
        }
        let Some(value) = gnome
            .read(pair.schema, pair.key)
            .and_then(|v| to_winpoint(pair.conv, &v))
        else {
            continue;
        };
        for path in pair.settings {
            if !same(at(settings, path), &value) {
                set_path(&mut patch, path, value.clone());
            }
        }
    }
    patch
}

/// GNOME's accent names and their colours (libadwaita 1.6; brown is Ubuntu's, Yaru: only where the installed
/// accent-color takes it, [`accent_write`]).
const ACCENTS: &[(&str, &str)] = &[
    ("blue", "#3584e4"),
    ("teal", "#2190a4"),
    ("green", "#3a944a"),
    ("yellow", "#c88800"),
    ("orange", "#ed5b00"),
    ("red", "#e62d42"),
    ("pink", "#d56199"),
    ("purple", "#9141ac"),
    ("slate", "#6f8396"),
    ("brown", "#986a44"),
];

fn oklab(hex: &str) -> Option<[f64; 3]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| {
        let c = f64::from(u8::from_str_radix(&h[i..i + 2], 16).ok()?) / 255.0;
        Some(if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        })
    };
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    Some([
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ])
}

/// The GNOME accent name nearest to a hex colour (OKLab distance; the lightness counts half, so a dark or light
/// variant of a hue stays that hue).
#[cfg(test)]
pub(crate) fn nearest_accent(hex: &str) -> Option<&'static str> {
    nearest_accent_in(hex, |_| true)
}

/// [`nearest_accent`] among the names `ok` takes.
pub(crate) fn nearest_accent_in(hex: &str, ok: impl Fn(&str) -> bool) -> Option<&'static str> {
    let c = oklab(hex)?;
    ACCENTS
        .iter()
        .filter(|(name, _)| ok(name))
        .filter_map(|(name, h)| {
            let a = oklab(h)?;
            let d = 0.25 * (c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2) + (c[2] - a[2]).powi(2);
            Some((*name, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
}

/// The look pair: the accent-color GNOME should get, when it differs (only while theme.syncSystem, and only a name
/// the installed key takes, while it is writable).
pub(crate) fn accent_write(settings: &Value, gnome: &dyn Gnome) -> Option<glib::Variant> {
    if settings["theme"]["syncSystem"] != true || !gnome.writable(INTERFACE, ACCENT_KEY) {
        return None;
    }
    let target = nearest_accent_in(settings["theme"]["accent"].as_str()?, |name| {
        gnome.accepts(INTERFACE, ACCENT_KEY, &name.to_variant())
    })?
    .to_variant();
    (gnome.read(INTERFACE, ACCENT_KEY)? != target).then_some(target)
}

fn accent_touched(changed: &[String]) -> bool {
    changed.iter().any(|c| {
        matches!(
            c.as_str(),
            "" | "theme" | "theme.accent" | "theme.syncSystem"
        )
    })
}

/// The mirror's own GNOME writes, in write order, for a few seconds: the watcher skips them (the origin tag).
static ECHO: Mutex<Vec<(String, String, Instant)>> = Mutex::new(Vec::new());
const ECHO_TTL: Duration = Duration::from_secs(5);

fn echo_id(schema: &str, key: &str) -> String {
    format!("{schema} {key}")
}

pub(crate) fn tag(schema: &str, key: &str, value: &glib::Variant) {
    let mut echo = ECHO.lock().unwrap_or_else(|e| e.into_inner());
    echo.retain(|(_, _, at)| at.elapsed() < ECHO_TTL);
    echo.push((
        echo_id(schema, key),
        value.print(true).to_string(),
        Instant::now(),
    ));
}

/// A write that did not happen leaves no tag (it would hide a later change to that value made elsewhere).
pub(crate) fn untag(schema: &str, key: &str, value: &glib::Variant) {
    let mut echo = ECHO.lock().unwrap_or_else(|e| e.into_inner());
    let (id, text) = (echo_id(schema, key), value.print(true).to_string());
    if let Some(index) = echo.iter().rposition(|(i, t, _)| *i == id && *t == text) {
        echo.remove(index);
    }
}

/// Whether the change of (schema, key) the watcher reads as `value` (the key's value when its handler runs) is the
/// mirror's own write. The newest tag with that value is consumed with every older tag of the key: those writes
/// are behind the value read now (their notifications were coalesced into this one, or come later and read the
/// same value), so none of them can hide a later change made elsewhere.
pub(crate) fn is_echo(schema: &str, key: &str, value: &glib::Variant) -> bool {
    let mut echo = ECHO.lock().unwrap_or_else(|e| e.into_inner());
    echo.retain(|(_, _, at)| at.elapsed() < ECHO_TTL);
    let (id, text) = (echo_id(schema, key), value.print(true).to_string());
    match echo.iter().rposition(|(i, t, _)| *i == id && *t == text) {
        Some(index) => {
            let mut n = 0;
            echo.retain(|(i, _, _)| {
                n += 1;
                *i != id || n > index + 1
            });
            true
        }
        None => false,
    }
}

/// A tagged write; the tag goes again when GNOME did not take it.
fn tagged(schema: &str, key: &str, value: &glib::Variant, write: impl FnOnce() -> bool) -> bool {
    tag(schema, key, value);
    let ok = write();
    if !ok {
        untag(schema, key, value);
    }
    ok
}

/// settings.json → GNOME for a change, on `gnome`; the number of keys written.
pub(crate) fn apply_with(settings: &Value, changed: &[String], gnome: &dyn Gnome) -> usize {
    let mut written = 0;
    for (pair, value) in gnome_writes(settings, changed, gnome) {
        if !gnome.writable(pair.schema, pair.key) {
            continue; // locked: GNOME's value stays
        }
        written += usize::from(tagged(pair.schema, pair.key, &value, || {
            gnome.write(pair.schema, pair.key, &value)
        }));
    }
    if accent_touched(changed)
        && let Some(value) = accent_write(settings, gnome)
    {
        written += usize::from(tagged(INTERFACE, ACCENT_KEY, &value, || {
            gnome.write_look(INTERFACE, ACCENT_KEY, &value)
        }));
    }
    written
}

/// Where the agreed values are kept (`<config>/gnome-mirror.json`, set by [`watch`] in the panel host): None in the
/// other hosts and in tests, which pass their record themselves.
static AGREED_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn pair_id(pair: &Pair) -> String {
    echo_id(pair.schema, pair.key)
}

/// The record entry of a pair whose sides agree now (every setting path maps to GNOME's value), else None.
fn agreement(pair: &Pair, settings: &Value, gnome: &dyn Gnome) -> Option<Value> {
    let current = gnome.read(pair.schema, pair.key)?;
    let mut winpoint = Map::new();
    for path in pair.settings {
        let value = at(settings, path);
        if to_gnome(pair.conv, value).is_none() || !agrees(pair.conv, value, &current) {
            return None;
        }
        winpoint.insert((*path).to_owned(), value.clone());
    }
    Some(json!({"gnome": current.print(true).to_string(), "winpoint": winpoint}))
}

/// Record, in `agreed`, every pair of `pairs` whose sides agree now; whether the record changed.
pub(crate) fn remember<'a>(
    agreed: &mut Value,
    pairs: impl Iterator<Item = &'a Pair>,
    settings: &Value,
    gnome: &dyn Gnome,
) -> bool {
    if !agreed["pairs"].is_object() {
        *agreed = json!({"version": 1, "pairs": {}});
    }
    let mut changed = false;
    for pair in pairs {
        if let Some(entry) = agreement(pair, settings, gnome) {
            let id = pair_id(pair);
            if agreed["pairs"][&id] != entry {
                agreed["pairs"][&id] = entry;
                changed = true;
            }
        }
    }
    changed
}

fn agreed_load(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|v| v["pairs"].is_object())
        .unwrap_or_else(|| json!({"version": 1, "pairs": {}}))
}

fn agreed_save(path: &Path, agreed: &Value) {
    let temporary = path.with_extension(format!("json.{}", std::process::id()));
    if std::fs::write(&temporary, format!("{agreed:#}\n")).is_ok() {
        let _ = std::fs::rename(&temporary, path);
    }
}

/// [`remember`] in the panel host's record file (one writer at a time: the effects hook and the watcher).
fn remember_in_file<'a>(
    pairs: impl Iterator<Item = &'a Pair>,
    settings: &Value,
    gnome: &dyn Gnome,
) {
    let file = AGREED_FILE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(path) = file.as_ref() else { return };
    let mut agreed = agreed_load(path);
    if remember(&mut agreed, pairs, settings, gnome) {
        agreed_save(path, &agreed);
    }
}

/// The start (module docs): the patch that brings settings.json to GNOME for every pair GNOME moved on since the
/// record (or that has no record), and the GNOME writes of the pairs only settings.json moved on.
pub(crate) fn start_plan(
    settings: &Value,
    gnome: &dyn Gnome,
    agreed: &Value,
) -> (Value, Vec<(&'static Pair, glib::Variant)>) {
    let mut patch = Value::Object(Map::new());
    let mut pushes = Vec::new();
    for pair in all_pairs() {
        let Some(current) = gnome.read(pair.schema, pair.key) else {
            continue;
        };
        let record = &agreed["pairs"][pair_id(pair)];
        let gnome_moved =
            !record.is_object() || record["gnome"].as_str() != Some(current.print(true).as_str());
        if gnome_moved {
            let Some(value) = to_winpoint(pair.conv, &current) else {
                continue;
            };
            for path in pair.settings {
                if !same(at(settings, path), &value) {
                    set_path(&mut patch, path, value.clone());
                }
            }
            continue;
        }
        // GNOME is where both last were: a settings path that moved goes to GNOME (the first one, as `touched`)
        let moved = pair
            .settings
            .iter()
            .find(|path| !same(at(settings, path), &record["winpoint"][**path]));
        if let Some(path) = moved
            && let Some(target) = to_gnome(pair.conv, at(settings, path))
            && !agrees(pair.conv, at(settings, path), &current)
        {
            pushes.push((pair, target));
        }
    }
    (patch, pushes)
}

/// The effects hook (settings_effects.rs `Effects::apply`, the panel host only): settings.json → GNOME after a
/// change, the agreed values of the pairs it touched, then batch A's rendering effects.
pub(crate) fn apply(settings: &Value, changed: &[String]) {
    let gnome = Live::new();
    if apply_with(settings, changed, &gnome) > 0 {
        gio::Settings::sync();
    }
    remember_in_file(
        all_pairs().filter(|pair| touched(pair, changed).is_some()),
        settings,
        &gnome,
    );
    render::effects(settings, changed);
}

pub(crate) type Snapshot = Arc<dyn Fn() -> Option<Value> + Send + Sync>;
pub(crate) type Patch = Arc<dyn Fn(Value) + Send + Sync>;
/// Run a closure on the settings while the settings lock is held (settings.rs `with_snapshot`, as every other
/// `gnome_look::note` caller does); false when the settings could not be read.
pub(crate) type Locked = Arc<dyn Fn(&mut dyn FnMut(&Value)) -> bool + Send + Sync>;

/// The GNOME → settings.json thread of the panel host (module docs); stops when dropped.
pub(crate) struct Watcher {
    stopped: Arc<AtomicBool>,
    context: glib::MainContext,
}

impl Watcher {
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.context.wakeup();
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn non_empty(patch: &Value) -> bool {
    patch.as_object().is_some_and(|o| !o.is_empty())
}

/// The start on `gnome` (module docs), with the record `agreed`: adopt what GNOME moved on, push what settings.json
/// moved on and the accent (under the settings lock), then record what both sides hold.
pub(crate) fn start_with(
    gnome: &dyn Gnome,
    agreed: &mut Value,
    snapshot: &dyn Fn() -> Option<Value>,
    patch: &dyn Fn(Value),
    locked: &dyn Fn(&mut dyn FnMut(&Value)) -> bool,
) {
    let Some(settings) = snapshot() else { return };
    let (adopt, pushes) = start_plan(&settings, gnome, agreed);
    if non_empty(&adopt) {
        patch(adopt);
    }
    let mut written = 0;
    for (pair, value) in pushes {
        if gnome.writable(pair.schema, pair.key) {
            written += usize::from(tagged(pair.schema, pair.key, &value, || {
                gnome.write(pair.schema, pair.key, &value)
            }));
        }
    }
    locked(&mut |settings| {
        if let Some(value) = accent_write(settings, gnome) {
            written += usize::from(tagged(INTERFACE, ACCENT_KEY, &value, || {
                gnome.write_look(INTERFACE, ACCENT_KEY, &value)
            }));
        }
    });
    if written > 0 {
        gio::Settings::sync();
    }
    if let Some(settings) = snapshot() {
        remember(agreed, all_pairs(), &settings, gnome);
    }
}

/// Start the watcher: the start ([`start_with`], with `<config>/gnome-mirror.json`), then follow every paired key.
pub(crate) fn watch(
    snapshot: Snapshot,
    patch: Patch,
    locked: Locked,
    agreed_file: PathBuf,
) -> Watcher {
    let stopped = Arc::new(AtomicBool::new(false));
    let context = glib::MainContext::new();
    let (thread_stop, thread_context) = (stopped.clone(), context.clone());
    std::thread::spawn(move || {
        let _ = thread_context.with_thread_default(|| {
            let gnome = Rc::new(Live::new());
            // the record file is set only after the start: the start's own settings patches reach `apply`, whose
            // record the start's last step makes anyway
            let mut agreed = agreed_load(&agreed_file);
            start_with(gnome.as_ref(), &mut agreed, &*snapshot, &*patch, &*locked);
            agreed_save(&agreed_file, &agreed);
            *AGREED_FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(agreed_file);
            let mut schemas: Vec<&str> = all_pairs().map(|p| p.schema).collect();
            schemas.sort_unstable();
            schemas.dedup();
            let mut handlers = Vec::new();
            for schema in schemas {
                let Some(settings) = gnome.settings(schema) else {
                    continue;
                };
                let (gnome, snapshot, patch) = (gnome.clone(), snapshot.clone(), patch.clone());
                let id = settings.connect_changed(None, move |s, key| {
                    if !all_pairs().any(|p| p.schema == schema && p.key == key) {
                        return;
                    }
                    if is_echo(schema, key, &s.value(key)) {
                        return;
                    }
                    let Some(current) = snapshot() else { return };
                    let change = winpoint_patch(&current, gnome.as_ref(), Some((schema, key)));
                    if non_empty(&change) {
                        patch(change);
                    }
                    if let Some(now) = snapshot() {
                        remember_in_file(
                            all_pairs().filter(|p| p.schema == schema && p.key == key),
                            &now,
                            gnome.as_ref(),
                        );
                    }
                });
                handlers.push((settings, id));
            }
            while !thread_stop.load(Ordering::Acquire) {
                thread_context.iteration(true);
            }
            for (settings, id) in handlers {
                settings.disconnect(id);
            }
        });
    });
    Watcher { stopped, context }
}
