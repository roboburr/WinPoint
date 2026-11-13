//! The event dialog, in the window like WinUI's ContentDialog (a smoke layer over the app, a rounded card with
//! its footer): title, calendar, "Hele dag", start and end (date picker with the mini month, time list in
//! quarter hours), location and notes; "Opslaan", "Annuleren" and, for an existing event, "Verwijderen".
//! Changing the start keeps the duration, as Windows does. GNOME events open read-only with "Sluiten".
use super::mini::MiniMonth;
use super::{App, app, defer};
use crate::config::{self, t};
use crate::dates::{self, DAY, Ymd};
use crate::glyphs;
use crate::store::{self, Event};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub enum Init {
    New { start: i64, end: i64, all_day: bool },
    Edit(Event),
}

struct TimePicker {
    dd: gtk::DropDown,
    values: RefCell<Vec<i64>>,
    h24: bool,
}

impl TimePicker {
    fn new(h24: bool) -> Rc<TimePicker> {
        let dd = gtk::DropDown::from_strings(&[]);
        dd.add_css_class("cal-dd");
        dd.add_css_class("cal-time");
        let p = Rc::new(TimePicker { dd, values: RefCell::new((0..96).map(|i| i * 15).collect()), h24 });
        p.rebuild();
        p
    }

    fn rebuild(&self) {
        let labels: Vec<String> = self.values.borrow().iter().map(|m| dates::fmt_time(*m, self.h24)).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.dd.set_model(Some(&gtk::StringList::new(&refs)));
    }

    fn set(&self, minute: i64) {
        let minute = minute.clamp(0, DAY - 1);
        let missing = !self.values.borrow().contains(&minute);
        if missing {
            let mut v = self.values.borrow_mut();
            v.push(minute);
            v.sort_unstable();
            drop(v);
            self.rebuild();
        }
        let idx = self.values.borrow().iter().position(|m| *m == minute).unwrap_or(0);
        self.dd.set_selected(idx as u32);
    }

    fn get(&self) -> i64 {
        let i = self.dd.selected() as usize;
        self.values.borrow().get(i).copied().unwrap_or(0)
    }
}

struct DatePicker {
    button: gtk::MenuButton,
    label: gtk::Label,
    popover: gtk::Popover,
    mini: Rc<MiniMonth>,
    day: Cell<Ymd>,
    on_change: RefCell<Option<Rc<dyn Fn()>>>,
}

impl DatePicker {
    fn new(a: &Rc<App>, day: Ymd) -> Rc<DatePicker> {
        let button = gtk::MenuButton::new();
        button.add_css_class("cal-datebtn");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.append(&label);
        row.append(&glyphs::glyph("calendar", 16));
        button.set_child(Some(&row));
        let popover = gtk::Popover::new();
        popover.add_css_class("cal-popover");
        popover.set_has_arrow(false);
        let mini = MiniMonth::new();
        mini.root.add_css_class("cal-pop-mini");
        popover.set_child(Some(&mini.root));
        button.set_popover(Some(&popover));
        let p = Rc::new(DatePicker {
            button,
            label,
            popover,
            mini,
            day: Cell::new(day),
            on_change: RefCell::new(None),
        });
        let fd = a.first_dow();
        let weak = Rc::downgrade(&p);
        p.mini.connect_pick(move |d| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(p) = weak.upgrade() {
                    p.popover.popdown();
                    p.set(d, fd);
                    let cb = p.on_change.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                }
            });
        });
        p.set(day, fd);
        p
    }

    fn set(&self, d: Ymd, fd: u32) {
        self.day.set(d);
        self.label.set_label(&dates::short_date(d, config::en()));
        self.mini.set((d.0, d.1), d, dates::today(), fd, None);
    }
}

struct Form {
    id: Option<String>,
    readonly: bool,
    title: gtk::Entry,
    cal: gtk::DropDown,
    cal_ids: Vec<String>,
    all_day: gtk::CheckButton,
    start_date: Rc<DatePicker>,
    end_date: Rc<DatePicker>,
    start_time: Rc<TimePicker>,
    end_time: Rc<TimePicker>,
    location: gtk::Entry,
    notes: gtk::TextView,
    error: gtk::Label,
    save: Option<gtk::Button>,
    /// The last valid duration in minutes, kept when the start moves.
    duration: Cell<i64>,
    syncing: Cell<bool>,
    fd: u32,
}

impl Form {
    fn start(&self) -> i64 {
        if self.all_day.is_active() {
            dates::at(self.start_date.day.get(), 0)
        } else {
            dates::at(self.start_date.day.get(), self.start_time.get())
        }
    }

    fn end(&self) -> i64 {
        if self.all_day.is_active() {
            dates::at(self.end_date.day.get(), 0) + DAY
        } else {
            dates::at(self.end_date.day.get(), self.end_time.get())
        }
    }

    fn set_end(&self, end: i64, all_day: bool) {
        self.syncing.set(true);
        if all_day {
            let last = dates::civil_from_days((end.div_euclid(DAY) - 1).max(self.start().div_euclid(DAY)));
            self.end_date.set(last, self.fd);
        } else {
            let (d, m) = dates::split(end);
            self.end_date.set(d, self.fd);
            self.end_time.set(m);
        }
        self.syncing.set(false);
    }

    /// The start moved: keep the duration.
    fn start_changed(&self) {
        if self.syncing.get() {
            return;
        }
        let all_day = self.all_day.is_active();
        let start = self.start();
        self.set_end(start + self.duration.get().max(if all_day { DAY } else { 0 }), all_day);
        self.validate();
    }

    fn end_changed(&self) {
        if self.syncing.get() {
            return;
        }
        let d = self.end() - self.start();
        if d >= 0 {
            self.duration.set(d);
        }
        self.validate();
    }

    fn all_day_changed(&self) {
        let on = self.all_day.is_active();
        self.start_time.dd.set_visible(!on);
        self.end_time.dd.set_visible(!on);
        if self.syncing.get() {
            return;
        }
        if on {
            // Whole days from the start day to the end day.
            let days = (self.end_date.day.get().0, self.end_date.day.get().1, self.end_date.day.get().2);
            let span = (dates::day_number(days) - dates::day_number(self.start_date.day.get())).max(0) + 1;
            self.duration.set(span * DAY);
        } else {
            self.syncing.set(true);
            self.start_time.set(9 * 60);
            self.end_date.set(self.start_date.day.get(), self.fd);
            self.end_time.set(10 * 60);
            self.syncing.set(false);
            self.duration.set(60);
        }
        self.validate();
    }

    fn validate(&self) -> bool {
        let ok = self.end() >= self.start();
        self.error.set_label(if ok { "" } else { t("Het einde ligt voor het begin.", "The end is before the start.") });
        self.error.set_visible(!ok);
        if let Some(s) = &self.save {
            s.set_sensitive(ok);
        }
        ok
    }

    fn event(&self) -> Event {
        let buf = self.notes.buffer();
        let notes = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
        let cal = self.cal_ids.get(self.cal.selected() as usize).cloned().unwrap_or_else(|| "personal".into());
        Event {
            id: self.id.clone().unwrap_or_else(store::new_id),
            title: self.title.text().trim().to_owned(),
            calendar: cal,
            all_day: self.all_day.is_active(),
            start: self.start(),
            end: self.end(),
            location: self.location.text().trim().to_owned(),
            notes: notes.trim_end().to_owned(),
            readonly: false,
        }
    }
}

fn field_row(glyph: &'static str, child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("cal-field");
    let g = glyphs::glyph(glyph, 16);
    g.add_css_class("cal-field-glyph");
    g.set_valign(gtk::Align::Start);
    g.set_margin_top(7);
    row.append(&g);
    child.as_ref().set_hexpand(true);
    row.append(child);
    row
}

pub fn open(a: &Rc<App>, init: Init) {
    let h24 = a.h24();
    let fd = a.first_dow();
    let (event, is_new) = match init {
        Init::New { start, end, all_day } => (
            Event {
                id: String::new(),
                title: String::new(),
                calendar: "personal".into(),
                all_day,
                start,
                end,
                location: String::new(),
                notes: String::new(),
                readonly: false,
            },
            true,
        ),
        Init::Edit(e) => (e, false),
    };
    let readonly = event.readonly;

    let smoke = gtk::Box::new(gtk::Orientation::Vertical, 0);
    smoke.add_css_class("cal-smoke");
    smoke.set_hexpand(true);
    smoke.set_vexpand(true);
    smoke.set_widget_name("cal-editor");
    // Clicks on the smoke stay there (WinUI keeps the dialog open).
    let eat = gtk::GestureClick::new();
    smoke.add_controller(eat);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("cal-dialog");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    card.set_size_request(480, -1);
    card.set_overflow(gtk::Overflow::Hidden);
    smoke.append(&card);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_vexpand(true);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.add_css_class("cal-dialog-body");
    scroll.set_child(Some(&body));
    card.append(&scroll);

    let heading = gtk::Label::new(Some(if readonly {
        t("Gebeurtenis", "Event")
    } else if is_new {
        t("Nieuwe gebeurtenis", "New event")
    } else {
        t("Gebeurtenis bewerken", "Edit event")
    }));
    heading.add_css_class("cal-dialog-title");
    heading.set_xalign(0.0);
    body.append(&heading);

    let title = gtk::Entry::new();
    title.set_widget_name("cal-editor-title");
    title.add_css_class("w11-entry");
    title.add_css_class("cal-title-entry");
    title.set_placeholder_text(Some(t("Een titel toevoegen", "Add a title")));
    title.set_text(&event.title);
    title.set_editable(!readonly);
    body.append(&title);

    // Calendar
    let cals: Vec<store::CalendarInfo> = if readonly {
        a.store.calendar(&event.calendar).into_iter().collect()
    } else {
        a.store.writable()
    };
    let names: Vec<String> = cals.iter().map(|c| c.name()).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cal = gtk::DropDown::from_strings(&name_refs);
    cal.add_css_class("cal-dd");
    cal.set_widget_name("cal-editor-calendar");
    let cal_ids: Vec<String> = cals.iter().map(|c| c.id.clone()).collect();
    if let Some(i) = cal_ids.iter().position(|id| *id == event.calendar) {
        cal.set_selected(i as u32);
    }
    cal.set_sensitive(!readonly);
    body.append(&field_row("calendar", &cal));

    // When
    let all_day = gtk::CheckButton::with_label(t("Hele dag", "All day"));
    all_day.add_css_class("w11-check");
    all_day.set_widget_name("cal-editor-allday");
    all_day.set_active(event.all_day);
    all_day.set_sensitive(!readonly);
    body.append(&field_row("clock", &all_day));

    let (sd, sm) = dates::split(event.start);
    let start_date = DatePicker::new(a, sd);
    let start_time = TimePicker::new(h24);
    start_time.set(sm);
    let (end_day, end_min) = if event.all_day {
        (dates::civil_from_days((event.end.div_euclid(DAY) - 1).max(event.start.div_euclid(DAY))), 10 * 60)
    } else {
        dates::split(event.end)
    };
    let end_date = DatePicker::new(a, end_day);
    let end_time = TimePicker::new(h24);
    end_time.set(end_min);
    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    let lbl = |s: &str| {
        let l = gtk::Label::new(Some(s));
        l.add_css_class("cal-field-label");
        l.set_xalign(0.0);
        l
    };
    grid.attach(&lbl(t("Begin", "Start")), 0, 0, 1, 1);
    grid.attach(&start_date.button, 1, 0, 1, 1);
    grid.attach(&start_time.dd, 2, 0, 1, 1);
    grid.attach(&lbl(t("Einde", "End")), 0, 1, 1, 1);
    grid.attach(&end_date.button, 1, 1, 1, 1);
    grid.attach(&end_time.dd, 2, 1, 1, 1);
    start_date.button.set_hexpand(true);
    end_date.button.set_hexpand(true);
    for w in [start_date.button.upcast_ref::<gtk::Widget>(), end_date.button.upcast_ref(), start_time.dd.upcast_ref(), end_time.dd.upcast_ref()] {
        w.set_sensitive(!readonly);
    }
    body.append(&field_row("calendar-today", &grid));

    let location = gtk::Entry::new();
    location.add_css_class("w11-entry");
    location.set_widget_name("cal-editor-location");
    location.set_placeholder_text(Some(t("Een locatie toevoegen", "Add a location")));
    location.set_text(&event.location);
    location.set_editable(!readonly);
    body.append(&field_row("location", &location));

    let notes = gtk::TextView::new();
    notes.add_css_class("cal-notes");
    notes.set_widget_name("cal-editor-notes");
    notes.set_wrap_mode(gtk::WrapMode::WordChar);
    notes.set_editable(!readonly);
    notes.set_cursor_visible(!readonly);
    notes.buffer().set_text(&event.notes);
    let notes_scroll = gtk::ScrolledWindow::new();
    notes_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    notes_scroll.set_min_content_height(88);
    notes_scroll.set_max_content_height(160);
    notes_scroll.set_propagate_natural_height(true);
    notes_scroll.add_css_class("cal-notes-frame");
    notes_scroll.set_child(Some(&notes));
    body.append(&field_row("notes", &notes_scroll));

    if readonly {
        let note = gtk::Label::new(Some(t(
            "Deze gebeurtenis komt uit een GNOME-agenda en kan hier niet worden gewijzigd.",
            "This event comes from a GNOME calendar and cannot be changed here.",
        )));
        note.add_css_class("cal-dialog-note");
        note.set_wrap(true);
        note.set_xalign(0.0);
        body.append(&note);
    }
    let error = gtk::Label::new(None);
    error.add_css_class("cal-dialog-error");
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.set_visible(false);
    body.append(&error);

    // Footer
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("cal-dialog-footer");
    let mut save = None;
    let cancel = gtk::Button::with_label(if readonly { t("Sluiten", "Close") } else { t("Annuleren", "Cancel") });
    cancel.add_css_class("w11-button");
    cancel.set_widget_name("cal-editor-cancel");
    cancel.set_hexpand(true);
    let delete = if !readonly && !is_new {
        let d = gtk::Button::new();
        d.add_css_class("w11-button");
        d.set_widget_name("cal-editor-delete");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::Center);
        row.append(&glyphs::glyph("delete", 16));
        row.append(&gtk::Label::new(Some(t("Verwijderen", "Delete"))));
        d.set_child(Some(&row));
        d.set_hexpand(true);
        footer.append(&d);
        Some(d)
    } else {
        None
    };
    if !readonly {
        let s = gtk::Button::with_label(t("Opslaan", "Save"));
        s.add_css_class("w11-button");
        s.add_css_class("w11-accent");
        s.set_widget_name("cal-editor-save");
        s.set_hexpand(true);
        footer.append(&s);
        save = Some(s);
    }
    footer.append(&cancel);
    footer.set_homogeneous(true);
    card.append(&footer);

    let form = Rc::new(Form {
        id: if is_new { None } else { Some(event.id.clone()) },
        readonly,
        title,
        cal,
        cal_ids,
        all_day,
        start_date,
        end_date,
        start_time,
        end_time,
        location,
        notes,
        error,
        save,
        duration: Cell::new((event.end - event.start).max(0)),
        syncing: Cell::new(false),
        fd,
    });
    form.start_time.dd.set_visible(!event.all_day);
    form.end_time.dd.set_visible(!event.all_day);

    // Wiring
    let f = Rc::downgrade(&form);
    *form.start_date.on_change.borrow_mut() = Some(Rc::new(move || {
        if let Some(f) = f.upgrade() {
            f.start_changed();
        }
    }));
    let f = Rc::downgrade(&form);
    *form.end_date.on_change.borrow_mut() = Some(Rc::new(move || {
        if let Some(f) = f.upgrade() {
            f.end_changed();
        }
    }));
    let f = Rc::downgrade(&form);
    form.start_time.dd.connect_selected_notify(move |_| {
        if let Some(f) = f.upgrade() {
            f.start_changed();
        }
    });
    let f = Rc::downgrade(&form);
    form.end_time.dd.connect_selected_notify(move |_| {
        if let Some(f) = f.upgrade() {
            f.end_changed();
        }
    });
    let f = Rc::downgrade(&form);
    form.all_day.connect_toggled(move |_| {
        if let Some(f) = f.upgrade() {
            f.all_day_changed();
        }
    });
    cancel.connect_clicked(|_| {
        if let Some(a) = app() {
            a.close_dialog();
        }
    });
    if let Some(s) = &form.save {
        let f = Rc::downgrade(&form);
        s.connect_clicked(move |_| {
            if let Some(f) = f.upgrade() {
                save_form(&f);
            }
        });
        let f = Rc::downgrade(&form);
        form.title.connect_activate(move |_| {
            if let Some(f) = f.upgrade() {
                save_form(&f);
            }
        });
    }
    if let Some(d) = delete {
        let f = Rc::downgrade(&form);
        d.connect_clicked(move |_| {
            let Some(f) = f.upgrade() else { return };
            let Some(id) = f.id.clone() else { return };
            let Some(a) = app() else { return };
            match a.remove_event(&id) {
                Ok(()) => a.close_dialog(),
                Err(e) => {
                    f.error.set_label(&format!("{}: {e}", t("Verwijderen is mislukt", "Could not delete")));
                    f.error.set_visible(true);
                }
            }
        });
    }
    form.validate();
    // The smoke layer owns the form (dropped with the dialog).
    let keep = form.clone();
    smoke.connect_destroy(move |_| {
        let _ = &keep;
    });

    a.show_dialog(smoke.upcast_ref());
    let focus = if readonly { cancel.clone().upcast::<gtk::Widget>() } else { form.title.clone().upcast() };
    defer(move |_| {
        focus.grab_focus();
    });
}

fn save_form(f: &Rc<Form>) {
    if f.readonly || !f.validate() {
        return;
    }
    let Some(a) = app() else { return };
    match a.commit(f.event()) {
        Ok(()) => a.close_dialog(),
        Err(e) => {
            f.error.set_label(&format!("{}: {e}", t("Opslaan is mislukt", "Could not save")));
            f.error.set_visible(true);
        }
    }
}
