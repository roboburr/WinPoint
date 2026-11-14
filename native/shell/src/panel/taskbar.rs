//! The 48 px taskbar strip (`git show 418ea594:vendor/ui/js/shell/taskbar.js`, shell.js, tray.js): a centred (or left,
//! per taskbar.alignment) group with Start, the search pill (taskbar.search box/icon/hidden), Task View and the
//! pinned + running apps, and on the right the notification area, the system button (-> quick settings), the
//! clock with the bell (-> notifications) and the show-desktop sliver. panel/mod.rs toggles "w11-glass" on the
//! strip while the extension blurs it (then the 70 % tint, else the solid colour).
mod apps;
mod system;
mod tray;
mod util;

use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[allow(unused_imports)]
pub use apps::own_app;

struct Bar {
    shell: Rc<Shell>,
    root: gtk::Overlay,
    center: gtk::CenterBox,
    group: gtk::Box,
    start_btn: gtk::Button,
    search_slot: gtk::Box,
    search_mode: RefCell<String>,
    taskview_btn: gtk::Button,
    sys: Rc<system::System>,
    align: RefCell<String>,
    lang: RefCell<String>,
    hooked: Cell<bool>,
}

thread_local! {
    static BAR: RefCell<Option<Rc<Bar>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<Bar>) -> R) -> Option<R> {
    let b = BAR.with(|x| x.borrow().clone());
    b.map(|b| f(&b))
}

pub fn build(shell: &Rc<Shell>) -> gtk::Widget {
    let root = gtk::Overlay::new();
    root.add_css_class("w11-taskbar");
    root.set_widget_name("tb-strip");
    let center = gtk::CenterBox::new();
    center.add_css_class("w11-tb-bar");
    center.set_shrink_center_last(false);
    root.set_child(Some(&center));
    // the strip's width decides how many app buttons fit
    let sizer = gtk::DrawingArea::new();
    sizer.set_can_target(false);
    sizer.connect_resize(|_, _, _| apps::queue_layout());
    root.add_overlay(&sizer);
    let strip: gtk::Widget = root.clone().upcast();

    // centre group: Start, search, Task View, apps
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    group.add_css_class("w11-tb-center");
    group.set_valign(gtk::Align::Center);

    let start_btn = gtk::Button::new();
    start_btn.add_css_class("w11-tb-btn");
    start_btn.add_css_class("w11-tb-start");
    start_btn.add_css_class("w11-tb-hit");
    start_btn.set_widget_name("tb-start");
    let logo = crate::icons::glyph("start-logo", 24);
    logo.add_css_class("w11-tb-ico");
    start_btn.set_child(Some(&logo));
    start_btn.connect_clicked(|_| {
        before_flyout();
        crate::start::toggle();
    });
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    let anchor = start_btn.downgrade();
    right.connect_pressed(move |g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        before_flyout();
        // Above the Start button, its left edge on the button's (Robin, 13-11: the menu opened at the far left).
        let x = anchor.upgrade().and_then(|b| b.root().and_then(|root| b.compute_bounds(&root))).map(|r| r.x() as i32);
        crate::flyouts::menus::open_winx(&crate::core::shell(), x, true);
    });
    start_btn.add_controller(right);

    let search_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    search_slot.add_css_class("w11-tb-search-slot");
    search_slot.set_valign(gtk::Align::Center);

    let taskview_btn = gtk::Button::new();
    taskview_btn.add_css_class("w11-tb-btn");
    taskview_btn.add_css_class("w11-tb-taskview");
    taskview_btn.add_css_class("w11-tb-hit");
    taskview_btn.set_widget_name("tb-taskview");
    let tv = gtk::DrawingArea::new();
    tv.add_css_class("w11-tb-ico");
    tv.set_content_width(24);
    tv.set_content_height(24);
    tv.set_halign(gtk::Align::Center);
    tv.set_valign(gtk::Align::Center);
    tv.set_draw_func(draw_taskview);
    taskview_btn.set_child(Some(&tv));
    taskview_btn.connect_clicked(|_| {
        before_flyout();
        crate::core::shell().flyouts().toggle("taskview");
    });

    let apps_box = apps::build(shell, &strip);
    group.append(&start_btn);
    group.append(&search_slot);
    group.append(&taskview_btn);
    group.append(&apps_box);

    // right: notification area, system button, clock, show desktop
    let tray_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tray_box.add_css_class("w11-tb-tray");
    tray_box.set_valign(gtk::Align::Fill);
    let area = tray::build(shell, &strip);
    let sys = system::build(shell, &strip);
    for w in [area.upcast_ref::<gtk::Widget>(), sys.sys_btn.upcast_ref(), sys.clock_btn.upcast_ref()] {
        w.set_valign(gtk::Align::Center);
        tray_box.append(w);
    }
    tray_box.append(&sys.desktop_btn);
    center.set_end_widget(Some(&tray_box));
    apps::set_layout_widgets(group.upcast_ref(), tray_box.upcast_ref());

    let bar = Rc::new(Bar {
        shell: shell.clone(),
        root: root.clone(),
        center,
        group,
        start_btn,
        search_slot,
        search_mode: RefCell::new(String::new()),
        taskview_btn,
        sys,
        align: RefCell::new(String::new()),
        lang: RefCell::new(shell.lang().to_owned()),
        hooked: Cell::new(false),
    });
    BAR.with(|x| *x.borrow_mut() = Some(bar.clone()));

    // right click on empty space: the taskbar's own menu
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    right.connect_pressed(|g, _, x, y| {
        let Some(w) = g.widget() else { return };
        if let Some(hit) = w.pick(x, y, gtk::PickFlags::DEFAULT) {
            let mut n = Some(hit);
            while let Some(c) = n {
                if c.has_css_class("w11-tb-hit") {
                    return; // a button's own menu
                }
                n = c.parent();
            }
        }
        g.set_state(gtk::EventSequenceState::Claimed);
        let (wx, _) = util::window_point(&w, x, y);
        before_flyout();
        crate::flyouts::menus::taskbar_menu(&crate::core::shell(), wx);
    });
    root.add_controller(right);

    shell.on("settings.changed", |_, _| {
        with(|b| b.apply_settings());
    });
    bar.apply_settings();

    // the flyout host exists once panel::install has the window: follow which flyout is open
    glib::idle_add_local_once(|| {
        with(|b| b.hook_flyouts());
    });
    root.upcast()
}

/// Before a flyout, Start or a menu opens: the window list goes away.
pub(crate) fn before_flyout() {
    apps::flyout_opened();
}

/// The tray changed size: the app buttons may have more or less room.
pub(crate) fn tray_resized() {
    apps::queue_layout();
}

/// Taakbeheer and Taakbalkinstellingen (the end of the system button's menu, as taskbar.js).
pub(crate) fn taskbar_items(shell: &Rc<Shell>) -> Vec<util::Item> {
    let s1 = shell.clone();
    let s2 = shell.clone();
    vec![
        util::item(shell.t("Taakbeheer", "Task Manager"), move || util::open_app(&s1, "taskmgr")),
        util::item(shell.t("Taakbalkinstellingen", "Taskbar settings"), move || util::open_settings(&s2, "taskbar")),
    ]
}

impl Bar {
    fn hook_flyouts(&self) {
        if self.hooked.get() {
            return;
        }
        let Some(host) = self.shell.flyouts.borrow().clone() else {
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), || {
                with(|b| b.hook_flyouts());
            });
            return;
        };
        self.hooked.set(true);
        host.on_change(|open| {
            with(|b| b.sync_active(open));
        });
        self.sync_active(host.current().is_some());
    }

    fn sync_active(&self, open: bool) {
        let cur = self.shell.flyouts.borrow().as_ref().and_then(|f| f.current());
        util::set_class(&self.taskview_btn, "is-active", cur == Some("taskview"));
        util::set_class(&self.start_btn, "is-active", cur == Some("winx"));
        util::set_class(&self.sys.sys_btn, "is-active", cur == Some("quick"));
        util::set_class(&self.sys.clock_btn, "is-active", cur == Some("notifications") || cur == Some("calendar"));
        tray::sync_active(cur == Some("tray"));
        if cur == Some("notifications") {
            system::notifications_opened();
        }
        if open {
            apps::flyout_opened();
        }
    }

    fn apply_settings(&self) {
        let shell = &self.shell;
        let lang = shell.lang().to_owned();
        let lang_changed = *self.lang.borrow() != lang;
        *self.lang.borrow_mut() = lang;

        // alignment: centred on the screen, or 8 px from the left edge
        let align = if shell.setting("taskbar.alignment").as_str() == Some("left") { "left" } else { "center" };
        if *self.align.borrow() != align {
            *self.align.borrow_mut() = align.to_owned();
            self.center.set_start_widget(None::<&gtk::Widget>);
            self.center.set_center_widget(None::<&gtk::Widget>);
            if align == "left" {
                self.group.set_margin_start(8);
                self.center.set_start_widget(Some(&self.group));
            } else {
                self.group.set_margin_start(0);
                self.center.set_center_widget(Some(&self.group));
            }
            util::set_class(&self.root, "is-left", align == "left");
        }
        self.taskview_btn.set_visible(shell.setting("taskbar.taskView") != false);
        self.start_btn.set_tooltip_text(Some("Start"));
        self.taskview_btn.set_tooltip_text(Some(&shell.t("Taakweergave", "Task view")));
        self.render_search(lang_changed);
        system::settings_changed();
        tray::settings_changed();
        apps::settings_changed(lang_changed);
        self.root.queue_draw();
    }

    fn render_search(&self, force: bool) {
        let v = self.shell.setting("taskbar.search");
        let mode = match &v {
            serde_json::Value::Bool(false) => "hidden".to_owned(),
            serde_json::Value::String(s) if !s.is_empty() => s.clone(),
            _ => "box".to_owned(),
        };
        if *self.search_mode.borrow() == mode && !force {
            return;
        }
        *self.search_mode.borrow_mut() = mode.clone();
        while let Some(c) = self.search_slot.first_child() {
            self.search_slot.remove(&c);
        }
        if mode == "hidden" {
            self.search_slot.set_visible(false);
            return;
        }
        self.search_slot.set_visible(true);
        let label = self.shell.t("Zoeken", "Search");
        let button = gtk::Button::new();
        button.set_widget_name("tb-search");
        button.add_css_class("w11-tb-hit");
        button.set_tooltip_text(Some(&label));
        if mode == "icon" {
            button.add_css_class("w11-tb-btn");
            button.add_css_class("w11-tb-searchicon");
            let g = crate::icons::glyph("search", 20);
            g.add_css_class("w11-tb-ico");
            button.set_child(Some(&g));
        } else {
            button.add_css_class("w11-tb-searchbox");
            if mode == "label" || mode == "iconLabel" {
                button.add_css_class("is-compact");
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.set_valign(gtk::Align::Center);
            let g = crate::icons::glyph("search", 16);
            g.add_css_class("w11-tb-searchbox-icon");
            row.append(&g);
            let l = gtk::Label::new(Some(&label));
            l.add_css_class("w11-tb-searchbox-label");
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&l);
            button.set_child(Some(&row));
        }
        button.connect_clicked(|_| {
            before_flyout();
            crate::start::open_search("");
        });
        self.search_slot.append(&button);
    }
}

/// The Win11 Task View glyph: a front pane over a lighter back pane (24 x 24).
fn draw_taskview(da: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32) {
    let c = da.color();
    let (r, g, b, a) = (c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
    let scale = (w.min(h) as f64) / 24.0;
    cr.scale(scale, scale);
    let rounded = |x: f64, y: f64, w: f64, h: f64, rad: f64| {
        cr.new_sub_path();
        cr.arc(x + w - rad, y + rad, rad, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + w - rad, y + h - rad, rad, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(x + rad, y + h - rad, rad, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(x + rad, y + rad, rad, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
    };
    cr.set_line_width(1.4);
    rounded(8.5, 3.5, 12.0, 11.0, 2.0);
    cr.set_source_rgba(r, g, b, a * 0.18);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(r, g, b, a);
    let _ = cr.stroke();
    rounded(3.5, 8.5, 12.0, 11.0, 2.0);
    let front = if crate::core::shell().dark() { 0x2b as f64 / 255.0 } else { 0xf6 as f64 / 255.0 };
    cr.set_source_rgba(front, front, front, 1.0);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(r, g, b, a);
    let _ = cr.stroke();
}
