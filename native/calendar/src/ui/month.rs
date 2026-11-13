//! The month view: weekday names over five or six weeks of cells (weeks from Monday in Dutch, Sunday in
//! English), the first of a month as "1 nov", today's number in the accent with a soft accent cell, the
//! selected day outlined, the other months' days dimmed. Each cell lists event chips (a colour dot, the start
//! time and the title; all-day events as tinted bars) as many as fit, then "+N meer". A click selects a day,
//! a double click starts an all-day event on it, the wheel moves a month.
use super::{App, View, app, defer};
use crate::config::{self, t};
use crate::dates::{self, DAY, Ymd};
use crate::glyphs;
use crate::store::{self, Event};
use crate::theme;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

/// Height of a cell's date row and of one chip row (chip + spacing), for the fit count.
const HEAD_H: f64 = 30.0;
const CHIP_H: f64 = 22.0;

pub fn build(a: &Rc<App>) -> gtk::Widget {
    let en = config::en();
    let fd = a.first_dow();
    let (sel, fit, today) = {
        let st = a.st.borrow();
        (st.sel, st.fit, st.today)
    };
    let (start, n) = a.view_range();
    let weeks = (n / 7).max(1) as i32;
    let from = dates::day_number(start) * DAY;
    let events = a.store.between(from, from + n as i64 * DAY);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("cal-month");
    root.set_vexpand(true);

    let head = gtk::Grid::new();
    head.add_css_class("cal-month-head");
    head.set_column_homogeneous(true);
    for i in 0..7u32 {
        let l = gtk::Label::new(Some(dates::day_name((fd + i) % 7, en)));
        l.add_css_class("cal-month-wd");
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_hexpand(true);
        head.attach(&l, i as i32, 0, 1, 1);
    }
    root.append(&head);

    let grid = gtk::Grid::new();
    grid.add_css_class("cal-month-grid");
    grid.set_row_homogeneous(true);
    grid.set_column_homogeneous(true);
    grid.set_hexpand(true);
    grid.set_vexpand(true);
    for i in 0..n as i64 {
        let d = dates::add_days(start, i);
        let day_from = dates::day_number(d) * DAY;
        let evs: Vec<&Event> = events.iter().filter(|e| e.overlaps(day_from, day_from + DAY)).collect();
        let c = cell(a, d, &evs, fit, today, sel, (i % 7) == 6);
        grid.attach(&c, (i % 7) as i32, (i / 7) as i32, 1, 1);
        a.cells.borrow_mut().push((d, c.upcast()));
    }

    // The grid does not ask for the room its chips want (the cells show what fits), so the window can shrink.
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
    scroller.set_child(Some(&grid));
    scroller.set_vexpand(true);
    scroller.set_hexpand(true);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroller));
    overlay.set_vexpand(true);
    let sensor = gtk::DrawingArea::new();
    sensor.set_can_target(false);
    sensor.set_hexpand(true);
    sensor.set_vexpand(true);
    sensor.connect_resize(move |_, _, h| {
        let cell_h = h as f64 / weeks as f64;
        let fit = (((cell_h - HEAD_H + 2.0) / CHIP_H).floor() as i32).max(0);
        let Some(a) = app() else { return };
        if a.st.borrow().fit == fit {
            return;
        }
        a.st.borrow_mut().fit = fit;
        glib::idle_add_local_once(|| {
            if let Some(a) = app()
                && a.st.borrow().view == View::Month
            {
                a.render_view();
            }
        });
    });
    overlay.add_overlay(&sensor);

    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    scroll.connect_scroll(|_, _, dy| {
        if dy > 0.0 {
            defer(|a| a.shift(1));
        } else if dy < 0.0 {
            defer(|a| a.shift(-1));
        }
        glib::Propagation::Stop
    });
    overlay.add_controller(scroll);
    root.append(&overlay);
    root.upcast()
}

fn cell(a: &Rc<App>, d: Ymd, evs: &[&Event], fit: i32, today: Ymd, sel: Ymd, last_col: bool) -> gtk::Box {
    let en = config::en();
    let c = gtk::Box::new(gtk::Orientation::Vertical, 2);
    c.add_css_class("cal-mcell");
    c.set_widget_name(&format!("cal-day-{}", dates::ymd_string(d)));
    c.set_overflow(gtk::Overflow::Hidden);
    if last_col {
        c.add_css_class("is-last-col");
    }
    if (d.0, d.1) != (sel.0, sel.1) {
        c.add_css_class("is-other");
    }
    if d == today {
        c.add_css_class("is-today");
    }
    if d == sel {
        c.add_css_class("is-selected");
    }
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    head.add_css_class("cal-mcell-head");
    let text = if d.2 == 1 { dates::day_month_short(d, en) } else { d.2.to_string() };
    let num = gtk::Label::new(Some(&text));
    num.add_css_class("cal-mday");
    num.set_halign(gtk::Align::Start);
    head.append(&num);
    c.append(&head);

    let total = evs.len() as i32;
    let (shown, more) = if total <= fit {
        (total, 0)
    } else {
        let s = (fit - 1).max(0);
        (s, total - s)
    };
    for e in evs.iter().take(shown as usize) {
        c.append(&chip(a, e, d));
    }
    if more > 0 {
        let m = gtk::Button::with_label(&format!("+{more} {}", t("meer", "more")));
        m.add_css_class("cal-more");
        m.set_halign(gtk::Align::Start);
        m.set_tooltip_text(Some(t("Alle gebeurtenissen van deze dag", "All events of this day")));
        m.connect_clicked(move |_| defer(move |a| a.open_day(d, View::Day)));
        c.append(&m);
    }

    let g = gtk::GestureClick::new();
    g.set_button(1);
    g.connect_pressed(move |_, n, _, _| {
        let Some(a) = app() else { return };
        if n == 2 {
            a.new_all_day(d);
        } else if n == 1 {
            a.select_in_place(d);
        }
    });
    c.add_controller(g);
    c
}

/// An event as a one-line chip: a dot, the start time (on its first day) and the title; all-day events and
/// events of 24 hours or more as a tinted bar in the calendar's colour.
pub fn chip(a: &Rc<App>, e: &Event, day: Ymd) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("cal-chip");
    b.add_css_class(&store::css_class(&e.calendar));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if e.is_banner() {
        b.add_css_class("is-banner");
    } else {
        row.append(&glyphs::dot(theme::color_rgba(&a.store.color_of(&e.calendar)), 8));
        if dates::split(e.start).0 == day {
            let time = gtk::Label::new(Some(&dates::fmt_time(e.start, a.h24())));
            time.add_css_class("cal-chip-time");
            row.append(&time);
        }
    }
    let title = gtk::Label::new(Some(&e.display_title()));
    title.add_css_class("cal-chip-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_width_chars(1);
    row.append(&title);
    b.set_child(Some(&row));
    b.set_tooltip_text(Some(&tooltip(a, e)));
    let id = e.id.clone();
    b.connect_clicked(move |_| {
        if let Some(a) = app() {
            a.open_event(&id);
        }
    });
    b
}

/// Title, when and where, for tooltips.
pub fn tooltip(a: &Rc<App>, e: &Event) -> String {
    let mut s = e.display_title();
    s.push('\n');
    s.push_str(&when(a, e));
    if !e.location.trim().is_empty() {
        s.push('\n');
        s.push_str(e.location.trim());
    }
    if e.readonly {
        s.push('\n');
        s.push_str(t("GNOME-agenda (alleen-lezen)", "GNOME calendar (read-only)"));
    }
    s
}

/// "vrijdag 13 november 2026, 09:00 - 10:00" and the like.
pub fn when(a: &Rc<App>, e: &Event) -> String {
    let en = config::en();
    let h24 = a.h24();
    let (sd, sm) = dates::split(e.start);
    if e.all_day {
        let last = dates::civil_from_days((e.end.div_euclid(DAY) - 1).max(e.start.div_euclid(DAY)));
        if last == sd {
            format!("{}, {}", dates::full_date(sd, en), t("hele dag", "all day"))
        } else {
            format!("{} \u{2013} {}", dates::short_date(sd, en), dates::short_date(last, en))
        }
    } else {
        let (ed, em) = dates::split(e.end);
        if ed == sd || (e.end == dates::at(dates::add_days(sd, 1), 0)) {
            let end = if ed == sd { dates::fmt_time(em, h24) } else { dates::fmt_time(0, h24) };
            format!("{}, {} \u{2013} {}", dates::full_date(sd, en), dates::fmt_time(sm, h24), end)
        } else {
            format!(
                "{} {} \u{2013} {} {}",
                dates::short_date(sd, en),
                dates::fmt_time(sm, h24),
                dates::short_date(ed, en),
                dates::fmt_time(em, h24)
            )
        }
    }
}
