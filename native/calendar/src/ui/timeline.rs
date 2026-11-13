//! The week and day views: day headers (short weekday over the date number, today's in the accent), a row of
//! all-day bars, and 24 hour rows of 48 px with the hour labels in a gutter; timed events are blocks placed by
//! start and end (overlapping ones side by side), today's column has the "now" line. A click selects a day, a
//! double click on an empty slot starts an event at that half hour, a click on a day header opens that day.
use super::{App, View, app, defer, month};
use crate::config::{self, t};
use crate::dates::{self, DAY, Ymd};
use crate::store::{self, Event};
use crate::theme;
use gtk::gdk;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

pub const HOUR_H: f64 = 48.0;
const GUTTER: i32 = 56;
const ALLDAY_MAX: usize = 3;

#[derive(Clone, Copy, Debug)]
struct Seg {
    idx: usize,
    /// Minutes from the day's midnight.
    s: i64,
    e: i64,
    lane: usize,
    lanes: usize,
}

pub fn build(a: &Rc<App>, first: Ymd, n: usize) -> gtk::Widget {
    let en = config::en();
    let (today, sel) = {
        let st = a.st.borrow();
        (st.today, st.sel)
    };
    let from = dates::day_number(first) * DAY;
    let to = from + n as i64 * DAY;
    let events = a.store.between(from, to);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("cal-timeline");
    root.set_vexpand(true);

    // ---- day headers
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    head.add_css_class("cal-tl-head");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_size_request(GUTTER, -1);
    head.append(&spacer);
    let heads = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    heads.set_homogeneous(true);
    heads.set_hexpand(true);
    for i in 0..n {
        let d = dates::add_days(first, i as i64);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.add_css_class("cal-tl-dayhead");
        col.set_widget_name(&format!("cal-dayhead-{}", dates::ymd_string(d)));
        if d == today {
            col.add_css_class("is-today");
        }
        if d == sel && n > 1 {
            col.add_css_class("is-selected");
        }
        let name = gtk::Label::new(Some(if n == 1 {
            dates::day_name(dates::weekday(d), en)
        } else {
            dates::day_short(dates::weekday(d), en)
        }));
        name.add_css_class("cal-tl-dname");
        name.set_xalign(0.0);
        let num = gtk::Label::new(Some(&d.2.to_string()));
        num.add_css_class("cal-tl-dnum");
        num.set_halign(gtk::Align::Start);
        col.append(&name);
        col.append(&num);
        if n > 1 {
            col.set_cursor_from_name(Some("pointer"));
            col.set_tooltip_text(Some(t("Deze dag openen", "Open this day")));
            let g = gtk::GestureClick::new();
            g.set_button(1);
            g.connect_released(move |_, _, _, _| defer(move |a| a.open_day(d, View::Day)));
            col.add_controller(g);
            a.cells.borrow_mut().push((d, col.clone().upcast()));
        }
        heads.append(&col);
    }
    head.append(&heads);
    root.append(&head);

    // ---- all-day row
    let allday = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    allday.add_css_class("cal-tl-allday");
    let label = gtk::Label::new(Some(t("Hele dag", "All day")));
    label.add_css_class("cal-tl-allday-label");
    label.set_size_request(GUTTER, -1);
    label.set_xalign(1.0);
    label.set_yalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    allday.append(&label);
    let cols = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    cols.set_homogeneous(true);
    cols.set_hexpand(true);
    for i in 0..n {
        let d = dates::add_days(first, i as i64);
        let day_from = dates::day_number(d) * DAY;
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.add_css_class("cal-tl-allday-col");
        let banners: Vec<&Event> =
            events.iter().filter(|e| e.is_banner() && e.overlaps(day_from, day_from + DAY)).collect();
        let shown = if banners.len() > ALLDAY_MAX { ALLDAY_MAX - 1 } else { banners.len() };
        for e in banners.iter().take(shown) {
            col.append(&month::chip(a, e, d));
        }
        if banners.len() > shown {
            let m = gtk::Button::with_label(&format!("+{} {}", banners.len() - shown, t("meer", "more")));
            m.add_css_class("cal-more");
            m.set_halign(gtk::Align::Start);
            m.connect_clicked(move |_| defer(move |a| a.open_day(d, View::Day)));
            col.append(&m);
        }
        let g = gtk::GestureClick::new();
        g.set_button(1);
        g.connect_pressed(move |_, presses, _, _| {
            if presses == 2
                && let Some(a) = app()
            {
                a.new_all_day(d);
            }
        });
        col.add_controller(g);
        cols.append(&col);
    }
    allday.append(&cols);
    root.append(&allday);

    // ---- hours
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.add_css_class("cal-tl-scroll");
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let gutter = gtk::Box::new(gtk::Orientation::Vertical, 0);
    gutter.add_css_class("cal-tl-gutter");
    gutter.set_size_request(GUTTER, -1);
    let h24 = a.h24();
    for h in 0..24 {
        let l = gtk::Label::new(Some(&if h == 0 { String::new() } else { dates::fmt_hour(h, h24) }));
        l.add_css_class("cal-tl-hour");
        l.set_size_request(GUTTER, HOUR_H as i32);
        l.set_xalign(1.0);
        l.set_yalign(0.0);
        gutter.append(&l);
    }
    body.append(&gutter);
    let days = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    days.set_homogeneous(true);
    days.set_hexpand(true);
    for i in 0..n {
        let d = dates::add_days(first, i as i64);
        days.append(&column(a, d, &events, d == today));
    }
    body.append(&days);
    scroller.set_child(Some(&body));

    // Scroll to where the user was, else to an hour before now (today in view) or to 07:30.
    let saved = a.st.borrow().scroll;
    let target = saved.unwrap_or_else(|| {
        let now = dates::now_minutes();
        if now >= from && now < to {
            ((now.rem_euclid(DAY) as f64 / 60.0) - 1.5).max(0.0) * HOUR_H
        } else {
            7.5 * HOUR_H
        }
    });
    let adj = scroller.vadjustment();
    let done = Rc::new(Cell::new(false));
    let d2 = done.clone();
    // Once the 24 hours are laid out, scroll after the allocation: a value set inside it moved the scrollbar
    // but not the viewport's child, and the day still showed 00:00.
    let full = 24.0 * HOUR_H;
    adj.connect_changed(move |adj| {
        if d2.get() || adj.page_size() <= 0.0 || adj.upper() <= adj.page_size() {
            return;
        }
        if adj.upper() - adj.page_size() >= target || adj.upper() >= full {
            d2.set(true);
            let adj = adj.clone();
            gtk::glib::idle_add_local_once(move || {
                adj.set_value(target.min(adj.upper() - adj.page_size()).max(0.0));
            });
        }
    });
    adj.connect_value_changed(move |adj| {
        if done.get()
            && let Some(a) = app()
        {
            a.st.borrow_mut().scroll = Some(adj.value());
        }
    });
    root.append(&scroller);
    root.upcast()
}

fn column(a: &Rc<App>, d: Ymd, events: &[Event], is_today: bool) -> gtk::Overlay {
    let col = gtk::Overlay::new();
    col.add_css_class("cal-tl-col");
    if is_today {
        col.add_css_class("is-today");
    }
    let base = gtk::DrawingArea::new();
    base.add_css_class("cal-tl-lines");
    base.set_content_height((24.0 * HOUR_H) as i32);
    base.set_hexpand(true);
    base.set_draw_func(move |area, cr, w, _| {
        let c = area.color();
        let w = w as f64;
        for hour in 0..24 {
            let y = hour as f64 * HOUR_H;
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
            cr.rectangle(0.0, y, w, 1.0);
            let _ = cr.fill();
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64 * 0.45);
            cr.rectangle(0.0, y + HOUR_H / 2.0, w, 1.0);
            let _ = cr.fill();
        }
        if is_today {
            let now = dates::now_minutes().rem_euclid(DAY);
            let y = (now as f64 / 60.0 * HOUR_H).round();
            let acc = theme::accent_rgba();
            cr.set_source_rgba(acc.red() as f64, acc.green() as f64, acc.blue() as f64, 1.0);
            cr.rectangle(0.0, y - 1.0, w, 2.0);
            let _ = cr.fill();
            cr.arc(4.0, y, 4.0, 0.0, 2.0 * PI);
            let _ = cr.fill();
        }
    });
    if is_today {
        a.now_areas.borrow_mut().push(base.downgrade());
    }
    col.set_child(Some(&base));

    let day_from = dates::day_number(d) * DAY;
    let day_to = day_from + DAY;
    let mut segs: Vec<Seg> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_banner() && e.overlaps(day_from, day_to))
        .map(|(idx, e)| Seg {
            idx,
            s: e.start.max(day_from) - day_from,
            e: e.end.min(day_to) - day_from,
            lane: 0,
            lanes: 1,
        })
        .collect();
    layout(&mut segs);
    let places: Rc<RefCell<Vec<(gtk::Widget, Seg)>>> = Rc::new(RefCell::new(Vec::new()));
    for seg in segs {
        let b = block(a, &events[seg.idx], &seg);
        col.add_overlay(&b);
        places.borrow_mut().push((b.upcast(), seg));
    }
    let base_weak = base.downgrade();
    col.connect_get_child_position(move |_, child| {
        let places = places.borrow();
        let (_, s) = places.iter().find(|(w, _)| w == child)?;
        let width = base_weak.upgrade().map(|b| b.width()).unwrap_or(0) as f64;
        // Leave a strip on the right free for clicking an empty slot, like Windows.
        let usable = (width - 8.0).max(12.0);
        let lane_w = usable / s.lanes as f64;
        let x = (s.lane as f64 * lane_w).round() as i32 + 1;
        let y = (s.s as f64 / 60.0 * HOUR_H).round() as i32 + 1;
        let h = (((s.e - s.s) as f64 / 60.0 * HOUR_H).round() as i32 - 2).max(20);
        Some(gdk::Rectangle::new(x, y, (lane_w.round() as i32 - 2).max(10), h))
    });

    let g = gtk::GestureClick::new();
    g.set_button(1);
    g.connect_pressed(move |_, presses, _, y| {
        let Some(a) = app() else { return };
        if presses == 2 {
            let minute = ((y / HOUR_H * 2.0).floor() as i64 * 30).clamp(0, DAY - 30);
            a.new_event_on(d, Some(minute));
        } else if presses == 1 {
            a.select_in_place(d);
        }
    });
    base.add_controller(g);
    col
}

/// Lanes for overlapping events: clusters of events that overlap (each drawn at least 25 minutes tall), each
/// event in the first lane free at its start, the cluster's width shared by its lanes.
fn layout(segs: &mut [Seg]) {
    segs.sort_by(|a, b| a.s.cmp(&b.s).then((b.e - b.s).cmp(&(a.e - a.s))));
    let vis_end = |s: &Seg| s.e.max(s.s + 25);
    let mut i = 0;
    while i < segs.len() {
        let mut lanes_end: Vec<i64> = Vec::new();
        let mut cluster_end = i64::MIN;
        let mut j = i;
        while j < segs.len() && (j == i || segs[j].s < cluster_end) {
            let ve = vis_end(&segs[j]);
            let lane = match lanes_end.iter().position(|end| *end <= segs[j].s) {
                Some(l) => l,
                None => {
                    lanes_end.push(i64::MIN);
                    lanes_end.len() - 1
                }
            };
            lanes_end[lane] = ve;
            segs[j].lane = lane;
            cluster_end = cluster_end.max(ve);
            j += 1;
        }
        let lanes = lanes_end.len().max(1);
        for s in segs.iter_mut().take(j).skip(i) {
            s.lanes = lanes;
        }
        i = j;
    }
}

fn block(a: &Rc<App>, e: &Event, seg: &Seg) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("cal-block");
    b.add_css_class(&store::css_class(&e.calendar));
    if e.readonly {
        b.add_css_class("is-readonly");
    }
    b.set_overflow(gtk::Overflow::Hidden);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
    v.set_valign(gtk::Align::Start);
    let h_px = (seg.e - seg.s) as f64 / 60.0 * HOUR_H;
    let h24 = a.h24();
    let title_text = if h_px < 36.0 {
        format!("{}, {}", e.display_title(), dates::fmt_time(e.start, h24))
    } else {
        e.display_title()
    };
    let title = gtk::Label::new(Some(&title_text));
    title.add_css_class("cal-block-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_width_chars(1);
    v.append(&title);
    if h_px >= 36.0 {
        let time = gtk::Label::new(Some(&format!(
            "{} \u{2013} {}",
            dates::fmt_time(e.start, h24),
            dates::fmt_time(e.end, h24)
        )));
        time.add_css_class("cal-block-time");
        time.set_xalign(0.0);
        time.set_ellipsize(gtk::pango::EllipsizeMode::End);
        time.set_width_chars(1);
        v.append(&time);
    }
    if h_px >= 54.0 && !e.location.trim().is_empty() {
        let loc = gtk::Label::new(Some(e.location.trim()));
        loc.add_css_class("cal-block-time");
        loc.set_xalign(0.0);
        loc.set_ellipsize(gtk::pango::EllipsizeMode::End);
        loc.set_width_chars(1);
        v.append(&loc);
    }
    b.set_child(Some(&v));
    b.set_tooltip_text(Some(&month::tooltip(a, e)));
    let id = e.id.clone();
    b.connect_clicked(move |_| {
        if let Some(a) = app() {
            a.open_event(&id);
        }
    });
    b
}
