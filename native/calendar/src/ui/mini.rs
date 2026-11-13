//! The mini month of the navigation pane (and of the editor's date pickers): "november 2026" with up / down
//! arrows, two-letter weekdays, six weeks of day buttons; today in an accent circle, the selected day ringed,
//! the days of the current week or day view softly filled, the other months' days dimmed. The mouse wheel
//! moves a month.
use super::icon_button;
use crate::config::{self, t};
use crate::dates::{self, Ymd};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct MiniMonth {
    pub root: gtk::Box,
    title: gtk::Label,
    grid: gtk::Grid,
    month: Cell<(i32, u32)>,
    sel: Cell<Ymd>,
    today: Cell<Ymd>,
    first_dow: Cell<u32>,
    hl: Cell<Option<(i64, i64)>>,
    on_pick: RefCell<Option<Rc<dyn Fn(Ymd)>>>,
    on_month: RefCell<Option<Rc<dyn Fn((i32, u32))>>>,
    me: RefCell<Weak<MiniMonth>>,
}

impl MiniMonth {
    pub fn new() -> Rc<MiniMonth> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("cal-mini");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        head.add_css_class("cal-mini-head");
        let title = gtk::Label::new(None);
        title.add_css_class("cal-mini-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        head.append(&title);
        let (up, _) = icon_button("arrow-up", 32, t("Vorige maand", "Previous month"));
        let (down, _) = icon_button("arrow-down", 32, t("Volgende maand", "Next month"));
        head.append(&up);
        head.append(&down);
        root.append(&head);
        let grid = gtk::Grid::new();
        grid.add_css_class("cal-mini-grid");
        grid.set_column_homogeneous(true);
        grid.set_row_spacing(2);
        root.append(&grid);
        let today = dates::today();
        let me = Rc::new(MiniMonth {
            root,
            title,
            grid,
            month: Cell::new((today.0, today.1)),
            sel: Cell::new(today),
            today: Cell::new(today),
            first_dow: Cell::new(1),
            hl: Cell::new(None),
            on_pick: RefCell::new(None),
            on_month: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *me.me.borrow_mut() = Rc::downgrade(&me);
        let weak = Rc::downgrade(&me);
        up.connect_clicked(move |_| {
            if let Some(m) = weak.upgrade() {
                m.step(-1);
            }
        });
        let weak = Rc::downgrade(&me);
        down.connect_clicked(move |_| {
            if let Some(m) = weak.upgrade() {
                m.step(1);
            }
        });
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let weak = Rc::downgrade(&me);
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(m) = weak.upgrade() {
                if dy > 0.0 {
                    m.step(1);
                } else if dy < 0.0 {
                    m.step(-1);
                }
            }
            glib::Propagation::Stop
        });
        me.root.add_controller(scroll);
        me.refresh();
        me
    }

    pub fn connect_pick(&self, f: impl Fn(Ymd) + 'static) {
        *self.on_pick.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_month(&self, f: impl Fn((i32, u32)) + 'static) {
        *self.on_month.borrow_mut() = Some(Rc::new(f));
    }

    /// Show `month` with `sel` selected; `hl` is a span of day numbers [from, to) to fill softly.
    pub fn set(&self, month: (i32, u32), sel: Ymd, today: Ymd, first_dow: u32, hl: Option<(i64, i64)>) {
        self.month.set(month);
        self.sel.set(sel);
        self.today.set(today);
        self.first_dow.set(first_dow);
        self.hl.set(hl);
        self.refresh();
    }

    fn step(&self, delta: i32) {
        let (y, m) = self.month.get();
        let next = dates::add_months(y, m, delta);
        self.month.set(next);
        self.refresh();
        let cb = self.on_month.borrow().clone();
        if let Some(cb) = cb {
            cb(next);
        }
    }

    fn refresh(&self) {
        let en = config::en();
        let (y, m) = self.month.get();
        self.title.set_label(&dates::month_title(y, m, en));
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        let fd = self.first_dow.get();
        for i in 0..7u32 {
            let l = gtk::Label::new(Some(dates::day_mini((fd + i) % 7, en)));
            l.add_css_class("cal-mini-wd");
            self.grid.attach(&l, i as i32, 0, 1, 1);
        }
        let start = dates::week_start((y, m, 1), fd);
        let today = self.today.get();
        let sel = self.sel.get();
        let hl = self.hl.get();
        for i in 0..42i64 {
            let d = dates::add_days(start, i);
            let b = gtk::Button::with_label(&d.2.to_string());
            b.add_css_class("cal-mini-day");
            b.set_size_request(32, 32);
            b.set_halign(gtk::Align::Center);
            b.set_valign(gtk::Align::Center);
            if (d.0, d.1) != (y, m) {
                b.add_css_class("is-other");
            }
            if d == today {
                b.add_css_class("is-today");
            }
            if d == sel {
                b.add_css_class("is-selected");
            }
            let n = dates::day_number(d);
            if let Some((from, to)) = hl
                && n >= from
                && n < to
            {
                b.add_css_class("in-range");
            }
            b.set_tooltip_text(Some(&dates::full_date(d, en)));
            let weak = self.me.borrow().clone();
            b.connect_clicked(move |_| {
                let Some(me) = weak.upgrade() else { return };
                let cb = me.on_pick.borrow().clone();
                if let Some(cb) = cb {
                    cb(d);
                }
            });
            self.grid.attach(&b, (i % 7) as i32, 1 + (i / 7) as i32, 1, 1);
        }
    }
}
