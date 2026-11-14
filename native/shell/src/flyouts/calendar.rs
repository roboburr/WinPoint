//! The calendar card of the notifications flyout (`git show 418ea594:vendor/ui/js/shell/notifcenter.js` buildCalendar):
//! a "woensdag 12 november" header that folds the card, the month grid with previous and next month, the first
//! weekday from settings.time.firstDayOfWeek, today in an accent circle, a selected day with a ring, and
//! keyboard moves (arrows, PageUp/PageDown, Home). `Calendar::new()` builds it; notifications.rs places it under
//! the notification card. The folded state is kept per user (the HTML kept it in localStorage).
//!
//! A day (click, Enter or Space) and the month heading open the Calendar app (native/calendar, win11-calendar) on
//! that date: the flyout closes and the backend's apps.launch starts io.github.win11shell.Calendar.desktop with the
//! URI win11-calendar:YYYY-MM-DD (that path carries GNOME's activation token; a running Calendar takes it over).
//!
//! Also the date helpers (civil days, Dutch and English names) the notification time stamps use.
use crate::core::Shell;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The calendar has no flyout of its own: it is part of `notifications`.
pub fn register(_shell: &Rc<Shell>) {}

/// The Calendar app's desktop entry (scripts/install.py, installed with win11-calendar).
const CALENDAR_APP: &str = "io.github.win11shell.Calendar.desktop";

/// "win11-calendar:2026-12-24", the URI the Calendar app opens on that day.
pub fn calendar_uri(d: Ymd) -> String {
    format!("win11-calendar:{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

/// Close the flyout and open the Calendar app on `d` (Robin: "zodra ik een datum klik dat het die datum opent").
pub fn open_in_app(d: Ymd) {
    let shell = crate::core::shell();
    shell.flyouts().close();
    shell.call("apps.launch", serde_json::json!({"id": CALENDAR_APP, "uris": [calendar_uri(d)]}), |result| {
        if let Err(error) = result {
            eprintln!("win11-shell: de agenda kon niet worden geopend: {error}");
        }
    });
}

pub type Ymd = (i32, u32, u32);

const NL_DAYS: [&str; 7] = ["zondag", "maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag"];
const EN_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const NL_DAYS_SHORT: [&str; 7] = ["zo", "ma", "di", "wo", "do", "vr", "za"];
const EN_DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const NL_MONTHS: [&str; 12] = [
    "januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november",
    "december",
];
const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const NL_MONTHS_SHORT: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
const EN_MONTHS_SHORT: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01 (proleptic Gregorian).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let (m, d) = (m as i64, d as i64);
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> Ymd {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

pub fn day_number(d: Ymd) -> i64 {
    days_from_civil(d.0, d.1, d.2)
}

/// 0 = Sunday ... 6 = Saturday.
pub fn weekday(d: Ymd) -> u32 {
    (day_number(d) + 4).rem_euclid(7) as u32
}

pub fn add_days(d: Ymd, n: i64) -> Ymd {
    civil_from_days(day_number(d) + n)
}

pub fn add_months(y: i32, m: u32, delta: i32) -> (i32, u32) {
    let idx = y * 12 + (m as i32 - 1) + delta;
    (idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1)
}

/// Today in local time.
pub fn today() -> Ymd {
    glib::DateTime::now_local()
        .map(|d| (d.year(), d.month().clamp(1, 12) as u32, d.day_of_month().clamp(1, 31) as u32))
        .unwrap_or((1970, 1, 1))
}

pub fn day_name(wd: u32, en: bool) -> &'static str {
    let i = (wd % 7) as usize;
    if en { EN_DAYS[i] } else { NL_DAYS[i] }
}

pub fn day_short(wd: u32, en: bool) -> &'static str {
    let i = (wd % 7) as usize;
    if en { EN_DAYS_SHORT[i] } else { NL_DAYS_SHORT[i] }
}

pub fn month_name(m: u32, en: bool) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if en { EN_MONTHS[i] } else { NL_MONTHS[i] }
}

pub fn month_short(m: u32, en: bool) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if en { EN_MONTHS_SHORT[i] } else { NL_MONTHS_SHORT[i] }
}

/// "zaterdag 25 oktober" / "Saturday 25 October".
pub fn long_date(d: Ymd, en: bool) -> String {
    format!("{} {} {}", day_name(weekday(d), en), d.2, month_name(d.1, en))
}

/// "zaterdag 25 oktober 2026" / "Saturday 25 October 2026".
pub fn full_date(d: Ymd, en: bool) -> String {
    format!("{} {}", long_date(d, en), d.0)
}

/// A Fluent glyph button (IconButton of components.js): transparent, a hover fill, `size` px square.
pub fn icon_button(glyph: &str, size: i32, icon_px: i32) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&crate::icons::glyph(glyph, icon_px)));
    button.add_css_class("nc-iconbtn");
    button.set_size_request(size, size);
    button.set_valign(gtk::Align::Center);
    button
}

pub fn set_label(widget: &impl IsA<gtk::Accessible>, text: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(text)]);
}

fn state_file() -> std::path::PathBuf {
    glib::user_state_dir().join("win11-gnome").join("native-calendar-collapsed")
}

fn read_collapsed() -> bool {
    std::fs::read_to_string(state_file()).is_ok_and(|s| s.trim() == "1")
}

fn write_collapsed(v: bool) {
    let path = state_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, if v { "1" } else { "0" });
}

fn first_day(shell: &Shell) -> u32 {
    let v = shell.setting("time.firstDayOfWeek");
    let n = v
        .as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
    n.map(|n| n.rem_euclid(7) as u32).unwrap_or(1)
}

pub struct Calendar {
    root: gtk::Box,
    header: gtk::Button,
    header_text: gtk::Label,
    chevron: gtk::Box,
    revealer: gtk::Revealer,
    body: gtk::Box,
    month_label: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    weekdays: gtk::Box,
    stack: gtk::Stack,
    view: Cell<(i32, u32)>,
    focus_day: Cell<Ymd>,
    selected: Cell<Option<Ymd>>,
    collapsed: Cell<bool>,
    days: RefCell<Vec<(Ymd, gtk::Button)>>,
    serial: Cell<u32>,
    listeners: RefCell<Vec<Rc<dyn Fn(bool)>>>,
    me: RefCell<Weak<Calendar>>,
}

impl Calendar {
    pub fn new() -> Rc<Calendar> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("nc-surface");
        root.add_css_class("fo-cal");
        root.set_overflow(gtk::Overflow::Hidden);

        let header = gtk::Button::new();
        header.add_css_class("fo-cal-header");
        let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let header_text = gtk::Label::new(None);
        header_text.add_css_class("nc-body");
        header_text.add_css_class("nc-strong");
        header_text.set_xalign(0.0);
        header_text.set_hexpand(true);
        header_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let chevron = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        chevron.add_css_class("fo-cal-chev");
        chevron.set_size_request(32, 32);
        chevron.set_valign(gtk::Align::Center);
        chevron.set_halign(gtk::Align::Center);
        header_box.append(&header_text);
        header_box.append(&chevron);
        header.set_child(Some(&header_box));

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("fo-cal-body");
        let monthrow = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        monthrow.add_css_class("fo-cal-monthrow");
        let month_label = gtk::Label::new(None);
        month_label.add_css_class("nc-body");
        month_label.add_css_class("nc-strong");
        month_label.set_xalign(0.0);
        // Only the text opens the app (a click handler below), so the free room goes to the arrows' side.
        month_label.set_halign(gtk::Align::Start);
        let prev = icon_button("chevron-up", 32, 12);
        let next = icon_button("chevron-down", 32, 12);
        prev.set_hexpand(true);
        prev.set_halign(gtk::Align::End);
        monthrow.append(&month_label);
        monthrow.append(&prev);
        monthrow.append(&next);
        let weekdays = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        weekdays.set_homogeneous(true);
        weekdays.add_css_class("fo-cal-weekdays");
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(true);
        stack.set_vhomogeneous(true);
        stack.set_transition_duration(250);
        let gridwrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        gridwrap.add_css_class("fo-cal-gridwrap");
        gridwrap.set_overflow(gtk::Overflow::Hidden);
        gridwrap.append(&stack);
        body.append(&monthrow);
        body.append(&weekdays);
        body.append(&gridwrap);

        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(250);
        revealer.set_child(Some(&body));
        root.append(&header);
        root.append(&revealer);

        let now = today();
        let cal = Rc::new(Calendar {
            root,
            header,
            header_text,
            chevron,
            revealer,
            body,
            month_label,
            prev,
            next,
            weekdays,
            stack,
            view: Cell::new((now.0, now.1)),
            focus_day: Cell::new(now),
            selected: Cell::new(None),
            collapsed: Cell::new(false),
            days: RefCell::new(Vec::new()),
            serial: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
            me: RefCell::new(Weak::new()),
        });
        *cal.me.borrow_mut() = Rc::downgrade(&cal);

        let weak = Rc::downgrade(&cal);
        cal.header.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.set_collapsed(!c.collapsed.get(), true);
            }
        });
        let weak = Rc::downgrade(&cal);
        cal.prev.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.shift_month(-1, false);
            }
        });
        let weak = Rc::downgrade(&cal);
        cal.next.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.shift_month(1, false);
            }
        });
        // The month heading opens the app on that month: today in the current month, else its first day.
        let month_click = gtk::GestureClick::new();
        month_click.set_button(gtk::gdk::BUTTON_PRIMARY);
        let weak = Rc::downgrade(&cal);
        month_click.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(c) = weak.upgrade() {
                let (y, m) = c.view.get();
                let now = today();
                open_in_app(if (now.0, now.1) == (y, m) { now } else { (y, m, 1) });
            }
        });
        cal.month_label.add_controller(month_click);
        cal.month_label.set_cursor_from_name(Some("pointer"));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&cal);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(c) = weak.upgrade() else { return glib::Propagation::Proceed };
            if c.key(key) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        gridwrap.add_controller(keys);

        cal.set_collapsed(read_collapsed(), false);
        cal.render();
        cal
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed.get()
    }

    /// The card's height once its fold animation is done (for the notification list's room).
    pub fn height(&self) -> i32 {
        let header = self.header.measure(gtk::Orientation::Vertical, 360).1;
        let body = if self.collapsed.get() { 0 } else { self.body.measure(gtk::Orientation::Vertical, 360).1 };
        header + body + 2
    }

    /// Called with the new folded state after the header was clicked.
    pub fn on_toggle(&self, f: impl Fn(bool) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    /// Back to this month, today focused, nothing selected (the flyout opens).
    pub fn reset(&self) {
        let now = today();
        self.view.set((now.0, now.1));
        self.focus_day.set(now);
        self.selected.set(None);
        self.render();
    }

    /// Unfold and put the keyboard on the focused day.
    pub fn focus(&self) {
        if self.collapsed.get() {
            self.set_collapsed(false, true);
        }
        if !self.focus_current() {
            self.header.grab_focus();
        }
    }

    /// Whether the keyboard is somewhere in the calendar.
    pub fn has_focus(&self) -> bool {
        let root = self.root.clone().upcast::<gtk::Widget>();
        self.root.root().and_then(|r| r.focus()).is_some_and(|f| f == root || f.is_ancestor(&root))
    }

    /// The header button (the notification list hands the keyboard to it with the arrow keys).
    pub fn header(&self) -> gtk::Button {
        self.header.clone()
    }

    pub fn render(&self) {
        self.render_dir(0);
    }

    fn set_collapsed(&self, v: bool, animated: bool) {
        self.collapsed.set(v);
        if animated {
            write_collapsed(v);
        }
        if v { self.root.add_css_class("is-collapsed") } else { self.root.remove_css_class("is-collapsed") }
        while let Some(child) = self.chevron.first_child() {
            self.chevron.remove(&child);
        }
        let icon = crate::icons::glyph(if v { "chevron-up" } else { "chevron-down" }, 12);
        icon.set_hexpand(true);
        icon.set_halign(gtk::Align::Center);
        self.chevron.append(&icon);
        self.revealer.set_transition_duration(if animated { 250 } else { 0 });
        self.revealer.set_reveal_child(!v);
        self.update_header_label();
        if animated {
            let listeners: Vec<_> = self.listeners.borrow().clone();
            for f in listeners {
                f(v);
            }
        }
    }

    fn update_header_label(&self) {
        let shell = crate::core::shell();
        let action = if self.collapsed.get() {
            shell.t("Agenda uitvouwen", "Expand calendar")
        } else {
            shell.t("Agenda samenvouwen", "Collapse calendar")
        };
        set_label(&self.header, &format!("{}. {action}", self.header_text.text()));
    }

    fn shift_month(&self, delta: i32, keep_focus: bool) {
        let (y, m) = self.view.get();
        let (ny, nm) = add_months(y, m, delta);
        self.view.set((ny, nm));
        if !keep_focus {
            self.focus_day.set((ny, nm, 1));
        }
        self.render_dir(delta.signum());
    }

    fn select(&self, d: Ymd) {
        self.selected.set(Some(d));
        self.focus_day.set(d);
        if (d.0, d.1) != self.view.get() {
            self.view.set((d.0, d.1));
        }
        self.render();
        self.focus_current();
    }

    fn focus_current(&self) -> bool {
        let focus = self.focus_day.get();
        let days = self.days.borrow();
        days.iter().find(|(d, _)| *d == focus).is_some_and(|(_, b)| b.grab_focus())
    }

    fn key(&self, key: gtk::gdk::Key) -> bool {
        use gtk::gdk::Key;
        let f = self.focus_day.get();
        let target = match key {
            Key::Left | Key::KP_Left => add_days(f, -1),
            Key::Right | Key::KP_Right => add_days(f, 1),
            Key::Up | Key::KP_Up => add_days(f, -7),
            Key::Down | Key::KP_Down => add_days(f, 7),
            Key::Page_Up | Key::KP_Page_Up => {
                let (y, m) = add_months(f.0, f.1, -1);
                (y, m, f.2.min(28))
            }
            Key::Page_Down | Key::KP_Page_Down => {
                let (y, m) = add_months(f.0, f.1, 1);
                (y, m, f.2.min(28))
            }
            Key::Home | Key::KP_Home => today(),
            _ => return false,
        };
        let (vy, vm) = self.view.get();
        let delta = (target.0 - vy) * 12 + target.1 as i32 - vm as i32;
        self.focus_day.set(target);
        if delta != 0 {
            self.view.set((target.0, target.1));
            self.render_dir(delta.signum());
        } else {
            self.render();
        }
        self.focus_current();
        true
    }

    fn render_dir(&self, dir: i32) {
        let shell = crate::core::shell();
        let en = shell.lang() == "en";
        let now = today();
        self.header_text.set_text(&long_date(now, en));
        self.update_header_label();
        let (vy, vm) = self.view.get();
        self.month_label.set_text(&format!("{} {vy}", month_name(vm, en)));
        self.month_label.set_tooltip_text(Some(&shell.t("Openen in Agenda", "Open in Calendar")));
        let prev = shell.t("Vorige maand", "Previous month");
        let next = shell.t("Volgende maand", "Next month");
        self.prev.set_tooltip_text(Some(&prev));
        set_label(&self.prev, &prev);
        self.next.set_tooltip_text(Some(&next));
        set_label(&self.next, &next);

        // weekday header: ma di wo do vr za zo / Mo Tu We ...
        let fd = first_day(&shell);
        while let Some(child) = self.weekdays.first_child() {
            self.weekdays.remove(&child);
        }
        for i in 0..7 {
            let short: String = day_short((fd + i) % 7, en).chars().take(2).collect();
            let label = gtk::Label::new(Some(&if en { short } else { short.to_lowercase() }));
            label.add_css_class("nc-body");
            self.weekdays.append(&label);
        }

        // 6 x 7 days
        let stack_widget = self.stack.clone().upcast::<gtk::Widget>();
        let had_focus = self.root.root().and_then(|r| r.focus()).is_some_and(|f| f.is_ancestor(&stack_widget));
        let grid = gtk::Grid::new();
        grid.set_column_homogeneous(true);
        grid.add_css_class("fo-cal-grid");
        let first: Ymd = (vy, vm, 1);
        let offset = (weekday(first) + 7 - fd) % 7;
        let start = add_days(first, -(offset as i64));
        let focus = self.focus_day.get();
        let selected = self.selected.get();
        let today_text = shell.t("vandaag", "today");
        let mut days = Vec::with_capacity(42);
        for r in 0..6i32 {
            for c in 0..7i32 {
                let d = add_days(start, (r * 7 + c) as i64);
                let button = gtk::Button::new();
                let label = gtk::Label::new(Some(&d.2.to_string()));
                button.set_child(Some(&label));
                button.add_css_class("fo-cal-day");
                button.set_size_request(38, 38);
                button.set_halign(gtk::Align::Center);
                button.set_valign(gtk::Align::Center);
                button.set_margin_top(2);
                button.set_margin_bottom(2);
                button.set_focusable(d == focus);
                let is_today = d == now;
                if (d.0, d.1) != (vy, vm) {
                    button.add_css_class("is-other");
                }
                if is_today {
                    button.add_css_class("is-today");
                }
                if selected == Some(d) {
                    button.add_css_class("is-selected");
                }
                let mut name = full_date(d, en);
                if is_today {
                    name.push_str(", ");
                    name.push_str(&today_text);
                }
                set_label(&button, &name);
                let weak = self.me.borrow().clone();
                button.connect_clicked(move |_| {
                    if let Some(c) = weak.upgrade() {
                        c.select(d);
                    }
                    open_in_app(d);
                });
                grid.attach(&button, c, r, 1, 1);
                days.push((d, button));
            }
        }
        *self.days.borrow_mut() = days;

        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        let old = self.stack.visible_child();
        self.stack.add_named(&grid, Some(&format!("m{serial}")));
        self.stack.set_transition_type(match dir {
            d if d > 0 => gtk::StackTransitionType::SlideUp,
            d if d < 0 => gtk::StackTransitionType::SlideDown,
            _ => gtk::StackTransitionType::None,
        });
        self.stack.set_visible_child(&grid);
        if let Some(old) = old {
            if dir == 0 {
                self.stack.remove(&old);
            } else {
                let stack = self.stack.downgrade();
                glib::timeout_add_local_once(Duration::from_millis(320), move || {
                    if let Some(stack) = stack.upgrade()
                        && old.parent().is_some_and(|p| p == stack.clone().upcast::<gtk::Widget>())
                        && stack.visible_child().as_ref() != Some(&old)
                    {
                        stack.remove(&old);
                    }
                });
            }
        }
        if had_focus {
            self.focus_current();
        }
    }
}
