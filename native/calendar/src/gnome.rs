//! Read-only events from GNOME's own calendars (Evolution Data Server: online accounts, GNOME Calendar), through
//! gnome-shell's calendar server on the session bus, the way gnome-shell's js/ui/calendar.js DBusEventSource
//! does it:
//!
//!   org.gnome.Shell.CalendarServer at /org/gnome/Shell/CalendarServer
//!     method   SetTimeRange(x since, x until, b force_reload)      Unix seconds
//!     signal   EventsAddedOrUpdated(a(ssxxa{sv}) events)           (id, summary, start, end, extras)
//!     signal   EventsRemoved(as ids)                               id prefixes
//!     signal   ClientDisappeared(s source_uid)                     drops "<source_uid>\n..."
//!     property HasCalendars b
//!
//! An id that does not end in "\n" is one occurrence of a recurring event: its parent's earlier occurrences
//! (the id up to the last "\n") are dropped first, as calendar.js does. When the service is missing the
//! calendar simply shows no GNOME events.
use crate::dates::{self, DAY};
use crate::store::{Event, REMOTE_ID};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

const NAME: &str = "org.gnome.Shell.CalendarServer";
const PATH: &str = "/org/gnome/Shell/CalendarServer";
const IFACE: &str = "org.gnome.Shell.CalendarServer";

pub struct Remote {
    conn: RefCell<Option<gio::DBusConnection>>,
    /// id -> event (civil minutes).
    pub events: RefCell<HashMap<String, Event>>,
    pub has_calendars: Cell<bool>,
    pub available: Cell<bool>,
    want: Cell<Option<(i64, i64)>>,
    sent: Cell<Option<(i64, i64)>>,
    on_change: RefCell<Option<Rc<dyn Fn()>>>,
    subs: RefCell<Vec<gio::SignalSubscription>>,
    notify_pending: Cell<bool>,
    me: RefCell<Weak<Remote>>,
}

impl Remote {
    pub fn new() -> Rc<Remote> {
        let r = Rc::new(Remote {
            conn: RefCell::new(None),
            events: RefCell::new(HashMap::new()),
            has_calendars: Cell::new(false),
            available: Cell::new(false),
            want: Cell::new(None),
            sent: Cell::new(None),
            on_change: RefCell::new(None),
            subs: RefCell::new(Vec::new()),
            notify_pending: Cell::new(false),
            me: RefCell::new(Weak::new()),
        });
        *r.me.borrow_mut() = Rc::downgrade(&r);
        r
    }

    /// Connect to the session bus; `on_change` runs (batched) whenever the GNOME events change.
    pub fn start(self: &Rc<Self>, on_change: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(on_change));
        let weak = Rc::downgrade(self);
        gio::bus_get(gio::BusType::Session, None::<&gio::Cancellable>, move |res| {
            let Some(me) = weak.upgrade() else { return };
            match res {
                Ok(conn) => me.connected(conn),
                Err(e) => eprintln!("win11-calendar: session bus: {e}"),
            }
        });
    }

    fn connected(self: &Rc<Self>, conn: gio::DBusConnection) {
        let mut subs = Vec::new();
        for member in ["EventsAddedOrUpdated", "EventsRemoved", "ClientDisappeared"] {
            let weak = Rc::downgrade(self);
            subs.push(conn.subscribe_to_signal(
                Some(NAME),
                Some(IFACE),
                Some(member),
                Some(PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |sig| {
                    if let Some(me) = weak.upgrade() {
                        me.signal(sig.signal_name, sig.parameters);
                    }
                },
            ));
        }
        let weak = Rc::downgrade(self);
        subs.push(conn.subscribe_to_signal(
            Some(NAME),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_| {
                if let Some(me) = weak.upgrade() {
                    me.read_has_calendars();
                }
            },
        ));
        *self.subs.borrow_mut() = subs;
        let appeared = Rc::downgrade(self);
        let vanished = Rc::downgrade(self);
        // The watch lasts as long as the process.
        let _watch = gio::bus_watch_name_on_connection(
            &conn,
            NAME,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, _| {
                if let Some(me) = appeared.upgrade() {
                    me.available.set(true);
                    me.read_has_calendars();
                    // A (re)started server knows nothing of our range.
                    me.sent.set(None);
                    me.send();
                }
            },
            move |_, _| {
                if let Some(me) = vanished.upgrade() {
                    me.available.set(false);
                    me.sent.set(None);
                    if !me.events.borrow().is_empty() {
                        me.events.borrow_mut().clear();
                        me.changed();
                    }
                }
            },
        );
        *self.conn.borrow_mut() = Some(conn);
        self.send();
    }

    /// Ask for the events of [since, until) (Unix seconds). `force` reloads even an unchanged range.
    pub fn request(self: &Rc<Self>, since: i64, until: i64, force: bool) {
        self.want.set(Some((since, until.max(since + 1))));
        if force {
            self.sent.set(None);
        }
        self.send();
    }

    fn send(self: &Rc<Self>) {
        let Some((since, until)) = self.want.get() else { return };
        if self.sent.get() == Some((since, until)) {
            return;
        }
        let Some(conn) = self.conn.borrow().clone() else { return };
        self.sent.set(Some((since, until)));
        // A new range starts from an empty cache, like calendar.js _loadEvents(true).
        if !self.events.borrow().is_empty() {
            self.events.borrow_mut().clear();
            self.changed();
        }
        let weak = Rc::downgrade(self);
        conn.call(
            Some(NAME),
            PATH,
            IFACE,
            "SetTimeRange",
            Some(&(since, until, true).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            15_000,
            None::<&gio::Cancellable>,
            move |res| {
                let Some(me) = weak.upgrade() else { return };
                match res {
                    Ok(_) => {
                        me.available.set(true);
                        me.read_has_calendars();
                    }
                    Err(_) => {
                        // No calendar server (or it failed): no GNOME events, quietly.
                        me.available.set(false);
                        if me.sent.get() == Some((since, until)) {
                            me.sent.set(None);
                        }
                        me.changed();
                    }
                }
            },
        );
    }

    fn read_has_calendars(self: &Rc<Self>) {
        let Some(conn) = self.conn.borrow().clone() else { return };
        let weak = Rc::downgrade(self);
        conn.call(
            Some(NAME),
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(IFACE, "HasCalendars").to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5_000,
            None::<&gio::Cancellable>,
            move |res| {
                let Some(me) = weak.upgrade() else { return };
                let has = res
                    .ok()
                    .and_then(|v| v.try_child_value(0))
                    .and_then(|v| v.as_variant())
                    .and_then(|v| v.get::<bool>())
                    .unwrap_or(false);
                if has != me.has_calendars.get() {
                    me.has_calendars.set(has);
                    me.changed();
                }
            },
        );
    }

    /// Whether the calendars list shows the GNOME row.
    pub fn present(&self) -> bool {
        self.available.get() && (self.has_calendars.get() || !self.events.borrow().is_empty())
    }

    fn signal(self: &Rc<Self>, name: &str, params: &glib::Variant) {
        let Some(arg) = params.try_child_value(0) else { return };
        match name {
            "EventsAddedOrUpdated" => {
                if !arg.is_container() {
                    return;
                }
                let mut changed = false;
                let mut handled: Vec<String> = Vec::new();
                let mut events = self.events.borrow_mut();
                for item in arg.iter() {
                    if !item.is_container() || item.n_children() < 4 {
                        continue;
                    }
                    let id = item.try_child_value(0).and_then(|v| v.str().map(str::to_owned));
                    let summary = item.try_child_value(1).and_then(|v| v.str().map(str::to_owned));
                    let start = item.try_child_value(2).and_then(|v| v.get::<i64>());
                    let end = item.try_child_value(3).and_then(|v| v.get::<i64>());
                    let (Some(id), Some(start)) = (id, start) else { continue };
                    let end = end.unwrap_or(start).max(start);
                    if !id.ends_with('\n') {
                        let parent = match id.rfind('\n') {
                            Some(i) => id[..=i].to_owned(),
                            None => String::new(),
                        };
                        if !parent.is_empty() && !handled.contains(&parent) {
                            events.retain(|k, _| !k.starts_with(&parent));
                            handled.push(parent);
                        }
                    }
                    let s = dates::unix_to_civil(start);
                    let e = dates::unix_to_civil(end).max(s);
                    let all_day = s.rem_euclid(DAY) == 0 && e.rem_euclid(DAY) == 0 && e > s;
                    events.insert(
                        id.clone(),
                        Event {
                            id,
                            title: summary.unwrap_or_default(),
                            calendar: REMOTE_ID.into(),
                            all_day,
                            start: s,
                            end: e,
                            location: String::new(),
                            notes: String::new(),
                            readonly: true,
                        },
                    );
                    changed = true;
                }
                drop(events);
                if changed {
                    self.changed();
                }
            }
            "EventsRemoved" => {
                let ids: Vec<String> = if arg.is_container() {
                    arg.iter().filter_map(|v| v.str().map(str::to_owned)).collect()
                } else {
                    Vec::new()
                };
                let mut changed = false;
                {
                    let mut events = self.events.borrow_mut();
                    for id in ids {
                        let before = events.len();
                        events.retain(|k, _| !k.starts_with(&id));
                        changed |= events.len() != before;
                    }
                }
                if changed {
                    self.changed();
                }
            }
            "ClientDisappeared" => {
                let Some(uid) = arg.str() else { return };
                let prefix = format!("{uid}\n");
                let before = self.events.borrow().len();
                self.events.borrow_mut().retain(|k, _| !k.starts_with(&prefix));
                if self.events.borrow().len() != before {
                    self.changed();
                }
            }
            _ => {}
        }
    }

    /// Tell the app, at most once per 80 ms (the server sends events in bursts).
    fn changed(&self) {
        if self.notify_pending.replace(true) {
            return;
        }
        let weak = self.me.borrow().clone();
        glib::timeout_add_local_once(Duration::from_millis(80), move || {
            let Some(me) = weak.upgrade() else { return };
            me.notify_pending.set(false);
            let cb = me.on_change.borrow().clone();
            if let Some(cb) = cb {
                cb();
            }
        });
    }
}
