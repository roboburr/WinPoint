//! Events and calendars. Local events live in ~/.local/share/win11-gnome/calendar/events.json (XDG_DATA_HOME
//! respected), written atomically (a temporary file in the same folder, fsync, rename). Read-only events from
//! GNOME's calendars (gnome.rs) sit beside them under the calendar id "gnome".
//!
//! events.json:
//!   { "version": 1,
//!     "calendars": [ { "id": "personal", "visible": true }, { "id": "x", "name": "Club", "color": "#e3008c" } ],
//!     "events": [ { "id": "...", "title": "...", "calendar": "personal", "allDay": false,
//!                   "start": "2026-11-13T09:00", "end": "2026-11-13T10:00", "location": "", "notes": "" },
//!                 { ..., "allDay": true, "start": "2026-11-20", "end": "2026-11-21" } ] }
//! Times are local wall-clock times; an all-day event's "end" is its last day (inclusive).
use crate::config::{self, t};
use crate::dates::{self, DAY};
use gtk::glib;
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

pub const REMOTE_ID: &str = "gnome";

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub calendar: String,
    pub all_day: bool,
    /// Civil minutes (dates.rs), start inclusive.
    pub start: i64,
    /// Civil minutes, end exclusive (an all-day event ends at the midnight after its last day).
    pub end: i64,
    pub location: String,
    pub notes: String,
    pub readonly: bool,
}

impl Event {
    /// Whether the event touches the half-open span [from, to) (a zero-length event counts at its start).
    pub fn overlaps(&self, from: i64, to: i64) -> bool {
        if self.end <= self.start {
            return self.start >= from && self.start < to;
        }
        self.start < to && self.end > from
    }

    /// Whether the event is drawn as an all-day bar (all day, or a timed event of 24 hours or more).
    pub fn is_banner(&self) -> bool {
        self.all_day || self.end - self.start >= DAY
    }

    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() { t("(Geen titel)", "(No title)").to_owned() } else { self.title.clone() }
    }
}

#[derive(Clone, Debug)]
pub struct CalendarInfo {
    pub id: String,
    pub name_nl: String,
    pub name_en: String,
    pub color: String,
    pub visible: bool,
    pub readonly: bool,
}

impl CalendarInfo {
    pub fn name(&self) -> String {
        if config::en() { self.name_en.clone() } else { self.name_nl.clone() }
    }

    /// The CSS class carrying this calendar's colour (theme.rs).
    pub fn css(&self) -> String {
        css_class(&self.id)
    }
}

pub fn css_class(id: &str) -> String {
    let clean: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("cal-c-{clean}")
}

fn builtin_calendars() -> Vec<CalendarInfo> {
    let mk = |id: &str, nl: &str, en: &str, color: &str, readonly: bool| CalendarInfo {
        id: id.into(),
        name_nl: nl.into(),
        name_en: en.into(),
        color: color.into(),
        visible: true,
        readonly,
    };
    vec![
        mk("personal", "Agenda", "Calendar", "#0078d4", false),
        mk("work", "Werk", "Work", "#00b294", false),
        mk("birthdays", "Verjaardagen", "Birthdays", "#e3008c", false),
        mk("family", "Gezin", "Family", "#ff8c00", false),
        mk(REMOTE_ID, "GNOME-agenda's", "GNOME calendars", "#8764b8", true),
    ]
}

pub struct Store {
    pub events: RefCell<Vec<Event>>,
    pub calendars: RefCell<Vec<CalendarInfo>>,
    pub remote: RefCell<HashMap<String, Event>>,
    /// GNOME's calendar server answers and has calendars (or sent events).
    pub remote_present: Cell<bool>,
    extra: RefCell<Map<String, Value>>,
}

pub fn events_path() -> PathBuf {
    config::data_dir().join("events.json")
}

impl Store {
    pub fn load() -> Store {
        let store = Store {
            events: RefCell::new(Vec::new()),
            calendars: RefCell::new(builtin_calendars()),
            remote: RefCell::new(HashMap::new()),
            remote_present: Cell::new(false),
            extra: RefCell::new(Map::new()),
        };
        let path = events_path();
        let Ok(bytes) = std::fs::read(&path) else { return store };
        let root: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                // Keep the unreadable file next to the new one instead of overwriting it on the next save.
                eprintln!("win11-calendar: {}: {e}", path.display());
                let _ = std::fs::copy(&path, path.with_extension("json.unreadable"));
                return store;
            }
        };
        if let Some(obj) = root.as_object() {
            let mut extra = Map::new();
            for (k, v) in obj {
                if k != "events" && k != "calendars" && k != "version" {
                    extra.insert(k.clone(), v.clone());
                }
            }
            *store.extra.borrow_mut() = extra;
        }
        if let Some(cals) = root["calendars"].as_array() {
            let mut list = store.calendars.borrow_mut();
            for c in cals {
                let Some(id) = c["id"].as_str().filter(|s| !s.is_empty()) else { continue };
                let visible = c["visible"].as_bool().unwrap_or(true);
                if let Some(known) = list.iter_mut().find(|k| k.id == id) {
                    known.visible = visible;
                    if let Some(color) = c["color"].as_str().filter(|s| valid_color(s)) {
                        known.color = color.to_owned();
                    }
                    continue;
                }
                let name = c["name"].as_str().unwrap_or(id).to_owned();
                let color = c["color"].as_str().filter(|s| valid_color(s)).unwrap_or("#0078d4").to_owned();
                let at = list.len().saturating_sub(1);
                list.insert(
                    at,
                    CalendarInfo {
                        id: id.to_owned(),
                        name_nl: name.clone(),
                        name_en: name,
                        color,
                        visible,
                        readonly: false,
                    },
                );
            }
        }
        if let Some(events) = root["events"].as_array() {
            let mut out = store.events.borrow_mut();
            for e in events {
                if let Some(ev) = event_from_json(e) {
                    out.push(ev);
                }
            }
        }
        store
    }

    pub fn save(&self) -> Result<(), String> {
        let mut root = self.extra.borrow().clone();
        root.insert("version".into(), json!(1));
        let cals: Vec<Value> = self
            .calendars
            .borrow()
            .iter()
            .map(|c| {
                if builtin_calendars().iter().any(|b| b.id == c.id) {
                    json!({ "id": c.id, "visible": c.visible })
                } else {
                    json!({ "id": c.id, "name": c.name_nl, "color": c.color, "visible": c.visible })
                }
            })
            .collect();
        root.insert("calendars".into(), Value::Array(cals));
        let events: Vec<Value> = self.events.borrow().iter().map(event_to_json).collect();
        root.insert("events".into(), Value::Array(events));
        let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
        write_atomic(&events_path(), text.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn calendar(&self, id: &str) -> Option<CalendarInfo> {
        self.calendars.borrow().iter().find(|c| c.id == id).cloned()
    }

    /// Calendars new events can go in.
    pub fn writable(&self) -> Vec<CalendarInfo> {
        self.calendars.borrow().iter().filter(|c| !c.readonly).cloned().collect()
    }

    pub fn color_of(&self, id: &str) -> String {
        self.calendar(id).map(|c| c.color).unwrap_or_else(|| "#0078d4".into())
    }

    pub fn is_visible(&self, id: &str) -> bool {
        self.calendars.borrow().iter().find(|c| c.id == id).map(|c| c.visible).unwrap_or(true)
    }

    pub fn set_visible(&self, id: &str, on: bool) {
        if let Some(c) = self.calendars.borrow_mut().iter_mut().find(|c| c.id == id) {
            c.visible = on;
        }
    }

    pub fn upsert(&self, e: Event) {
        let mut events = self.events.borrow_mut();
        if let Some(slot) = events.iter_mut().find(|x| x.id == e.id) {
            *slot = e;
        } else {
            events.push(e);
        }
    }

    pub fn delete(&self, id: &str) {
        self.events.borrow_mut().retain(|e| e.id != id);
    }

    /// A local or GNOME event by id.
    pub fn find(&self, id: &str) -> Option<Event> {
        if let Some(e) = self.events.borrow().iter().find(|e| e.id == id) {
            return Some(e.clone());
        }
        self.remote.borrow().get(id).cloned()
    }

    /// Visible events touching [from, to), banners first, then by start time and longest first.
    pub fn between(&self, from: i64, to: i64) -> Vec<Event> {
        let mut out: Vec<Event> = Vec::new();
        for e in self.events.borrow().iter() {
            if e.overlaps(from, to) && self.is_visible(&e.calendar) {
                out.push(e.clone());
            }
        }
        if self.remote_present.get() && self.is_visible(REMOTE_ID) {
            for e in self.remote.borrow().values() {
                if e.overlaps(from, to) {
                    out.push(e.clone());
                }
            }
        }
        sort_events(&mut out);
        out
    }
}

pub fn sort_events(list: &mut [Event]) {
    list.sort_by(|a, b| {
        b.is_banner()
            .cmp(&a.is_banner())
            .then(a.start.cmp(&b.start))
            .then((b.end - b.start).cmp(&(a.end - a.start)))
            .then(a.title.cmp(&b.title))
    });
}

pub fn new_id() -> String {
    let micros = glib::real_time();
    format!("evt-{micros:x}-{:08x}", glib::random_int())
}

fn valid_color(s: &str) -> bool {
    let h = s.trim_start_matches('#');
    s.starts_with('#') && (h.len() == 6 || h.len() == 3) && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn event_from_json(e: &Value) -> Option<Event> {
    let id = e["id"].as_str().filter(|s| !s.is_empty())?.to_owned();
    let all_day = e["allDay"].as_bool().unwrap_or(false);
    let start = dates::parse_civil(e["start"].as_str()?)?;
    let end_raw = e["end"].as_str().and_then(dates::parse_civil);
    let (start, end) = if all_day {
        let first = start.div_euclid(DAY);
        // "end" is the last day, inclusive.
        let last = end_raw.map(|m| m.div_euclid(DAY)).unwrap_or(first).max(first);
        (first * DAY, (last + 1) * DAY)
    } else {
        let end = end_raw.unwrap_or(start + 60).max(start);
        (start, end)
    };
    Some(Event {
        id,
        title: e["title"].as_str().unwrap_or("").to_owned(),
        calendar: e["calendar"].as_str().filter(|s| !s.is_empty() && *s != REMOTE_ID).unwrap_or("personal").to_owned(),
        all_day,
        start,
        end,
        location: e["location"].as_str().unwrap_or("").to_owned(),
        notes: e["notes"].as_str().unwrap_or("").to_owned(),
        readonly: false,
    })
}

fn event_to_json(e: &Event) -> Value {
    let (start, end) = if e.all_day {
        let first = dates::civil_from_days(e.start.div_euclid(DAY));
        let last = dates::civil_from_days((e.end.div_euclid(DAY) - 1).max(e.start.div_euclid(DAY)));
        (dates::ymd_string(first), dates::ymd_string(last))
    } else {
        (dates::civil_string(e.start), dates::civil_string(e.end))
    };
    json!({
        "id": e.id,
        "title": e.title,
        "calendar": e.calendar,
        "allDay": e.all_day,
        "start": start,
        "end": end,
        "location": e.location,
        "notes": e.notes,
    })
}

/// Write a file so that readers see the old or the new content, never half of it.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}
