//! The 48 px taskbar strip (`git show 418ea594:vendor/ui/js/shell/taskbar.js`, shell.js, tray.js): a centred (or left,
//! per taskbar.alignment) group with Start, the search pill (taskbar.search box/icon/hidden), Task View and the
//! pinned + running apps, and on the right the notification area, the system button (-> quick settings), the
//! clock with the bell (-> notifications) and the show-desktop sliver. panel/mod.rs toggles "w11-glass" on the
//! strip while the extension blurs it (then the 70 % tint, else the solid colour).
mod apps;
mod input;
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
/// The desktop in use and which windows show on it (taskbar.allDesktops here, taskView.allDesktops in Taakweergave).
pub(crate) use apps::{desktop_in_use, shows_on_desktop};

struct Bar {
    shell: Rc<Shell>,
    root: gtk::Overlay,
    center: gtk::CenterBox,
    group: gtk::Box,
    start_btn: gtk::Button,
    search_slot: gtk::Box,
    search_mode: RefCell<String>,
    search_text: RefCell<Option<gtk::Text>>,
    search_sync: Cell<bool>,
    taskview_btn: gtk::Button,
    sys: Rc<system::System>,
    _input: Rc<input::Input>,
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
    let input = input::build(shell, &strip);
    for w in [area.upcast_ref::<gtk::Widget>(), input.button.upcast_ref(), sys.sys_btn.upcast_ref(), sys.clock_btn.upcast_ref()] {
        w.set_valign(gtk::Align::Center);
        tray_box.append(w);
    }
    tray_box.append(&sys.desktop_btn);
    // the tray's size once GTK has allocated it (an icon, the clock, the bell's count) decides the room too
    let tray_slot = gtk::Overlay::new();
    tray_slot.set_child(Some(&tray_box));
    let tray_sizer = gtk::DrawingArea::new();
    tray_sizer.set_can_target(false);
    tray_sizer.connect_resize(|_, _, _| apps::queue_layout());
    tray_slot.add_overlay(&tray_sizer);
    center.set_end_widget(Some(&tray_slot));
    apps::set_layout_widgets(group.upcast_ref(), tray_box.upcast_ref());

    let bar = Rc::new(Bar {
        shell: shell.clone(),
        root: root.clone(),
        center,
        group,
        start_btn,
        search_slot,
        search_mode: RefCell::new(String::new()),
        search_text: RefCell::new(None),
        search_sync: Cell::new(false),
        taskview_btn,
        sys,
        _input: input,
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

    shell.on("settings.changed", |_, data| {
        with(|b| b.apply_settings(data));
    });
    bar.apply_settings(&serde_json::Value::Null);

    // the flyout host exists once panel::install has the window: follow which flyout is open
    glib::idle_add_local_once(|| {
        with(|b| b.hook_flyouts());
    });
    root.upcast()
}

/// The taskbar of another monitor (panel/strips.rs): the strip of the main taskbar with only its app buttons, those of
/// the windows on that monitor (apps.rs `shows`), aligned by taskbar.alignment as the main one. A right click on
/// empty space opens the taskbar's own menu above it, and any press closes the flyout the main taskbar may have open
/// (a click on a dock takes no focus, so the flyout's light dismiss would not see it).
pub fn build_secondary(shell: &Rc<Shell>, monitor: i64) -> gtk::Widget {
    let root = gtk::Overlay::new();
    root.add_css_class("w11-taskbar");
    root.set_widget_name(&format!("tb-strip-m{monitor}"));
    let center = gtk::CenterBox::new();
    center.add_css_class("w11-tb-bar");
    center.set_shrink_center_last(false);
    root.set_child(Some(&center));
    let strip: gtk::Widget = root.clone().upcast();
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    group.add_css_class("w11-tb-center");
    group.set_valign(gtk::Align::Center);
    let (apps_box, apps) = apps::build_for(shell, &strip, apps::Role::Secondary(monitor));
    group.append(&apps_box);
    let sizer = gtk::DrawingArea::new();
    sizer.set_can_target(false);
    let weak = Rc::downgrade(&apps);
    sizer.connect_resize(move |_, _, _| {
        if let Some(a) = weak.upgrade() {
            a.queue_layout();
        }
    });
    root.add_overlay(&sizer);
    apps.set_bar(&center, &group);

    let press = gtk::GestureClick::new();
    press.set_button(0);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    press.connect_pressed(|_, _, _, _| {
        let host = crate::core::shell().flyouts.borrow().clone();
        if let Some(host) = host.filter(|h| h.current().is_some()) {
            host.close();
        }
    });
    root.add_controller(press);
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    let weak = Rc::downgrade(&apps);
    right.connect_pressed(move |g, _, x, y| {
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
        apps::flyout_opened();
        let menu = crate::flyouts::menus::taskbar_menu_popover(&crate::core::shell(), &w, x.round() as i32);
        // no window list over it while it is open (the main taskbar's menu is a flyout, which does the same)
        if let Some(a) = weak.upgrade() {
            a.hold_previews(&menu);
        }
    });
    root.add_controller(right);
    strip
}

/// The extension adopted the panel again (after the lock screen): every taskbar tells it where its buttons are.
pub fn resend_icon_rects() {
    apps::resend_icon_rects();
}

/// Every taskbar's app buttons again (the taskbars of the other monitors came or went, or the setting of which
/// windows they show changed).
pub fn render_all_apps() {
    apps::render_all();
}

/// The taskbar of `monitor` went: its buttons are no longer kept up to date.
pub fn drop_secondary(monitor: i64) {
    apps::unregister(monitor);
}

/// Before a flyout, Start or a menu opens: the window list goes away.
pub(crate) fn before_flyout() {
    apps::flyout_opened();
}

/// Query updates from Start never move the taskbar caret or feed the same change back into Start.
pub(crate) fn mirror_search_query(query: &str) {
    with(|bar| {
        let text = bar.search_text.borrow().clone();
        if let Some(text) = text && text.text() != query {
            bar.search_sync.set(true);
            text.set_text(query);
            text.set_position(-1);
            bar.search_sync.set(false);
        }
    });
}

/// Closing search must not leave GtkWindow's remembered focus on the editable: merely reactivating the dock
/// for an unrelated button would otherwise emit focus-enter and open Search before that button is clicked.
pub(crate) fn finish_search_input() {
    mirror_search_query("");
    with(|bar| {
        let text = bar.search_text.borrow().clone();
        if let Some(text) = text
            && let Some(window) = text.root().and_then(|root| root.downcast::<gtk::Window>().ok())
            && gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus|
                focus == text.clone().upcast::<gtk::Widget>() || focus.is_ancestor(text.upcast_ref::<gtk::Widget>()))
        {
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        }
    });
}

/// The tray's icons changed: the app buttons may have more or less room. At once (the layout measures the tray),
/// so the group changes in the same frame as the tray.
pub(crate) fn tray_resized() {
    apps::layout_now();
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
            crate::start::dismiss_taskbar_search(false);
            apps::flyout_opened();
        }
    }

    /// A settings.changed (`data`, core::touches; Null: all): each part renders for its own keys only, so a write
    /// of another one (the host's taskbar.tray.known for a new tray icon) leaves the strip as it is.
    fn apply_settings(&self, data: &serde_json::Value) {
        let shell = &self.shell;
        let lang = shell.lang().to_owned();
        let lang_changed = *self.lang.borrow() != lang;
        *self.lang.borrow_mut() = lang;

        if lang_changed || crate::core::touches(data, &["taskbar.alignment", "taskbar.taskView", "taskbar.search"]) {
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
            // the group's own width changed: how many app buttons fit
            apps::queue_layout();
        }
        system::settings_changed(data, lang_changed);
        tray::settings_changed(data, lang_changed);
        apps::settings_changed(data, lang_changed);
        if crate::core::touches(data, &["theme"]) {
            self.root.queue_draw();
        }
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
        // Rebuilding/removing an editable must not strand its keyboard role or a stale query.
        crate::start::dismiss_taskbar_search(true);
        *self.search_text.borrow_mut() = None;
        while let Some(c) = self.search_slot.first_child() {
            self.search_slot.remove(&c);
        }
        if mode == "hidden" {
            self.search_slot.set_visible(false);
            return;
        }
        self.search_slot.set_visible(true);
        let label = self.shell.t("Zoeken", "Search");
        if mode == "box" {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("w11-tb-searchbox");
            row.add_css_class("w11-tb-hit");
            row.set_widget_name("tb-search");
            // Only GtkText expands inside the fixed pill; do not propagate that request to the centred group.
            row.set_hexpand(false);
            row.set_valign(gtk::Align::Center);
            row.append(&crate::icons::glyph("search", 16));
            let text = gtk::Text::new();
            text.set_widget_name("tb-search-input");
            text.add_css_class("w11-tb-searchbox-input");
            text.set_placeholder_text(Some(&label));
            text.set_hexpand(true);
            text.set_width_chars(1);
            text.set_max_width_chars(1);
            row.append(&text);
            let clear = gtk::Button::new();
            clear.set_widget_name("tb-search-clear");
            clear.add_css_class("w11-tb-searchbox-clear");
            clear.set_focusable(false);
            clear.set_tooltip_text(Some(&self.shell.t("Wissen", "Clear")));
            clear.set_child(Some(&crate::icons::glyph("close", 12)));
            clear.set_visible(false);
            row.append(&clear);
            *self.search_text.borrow_mut() = Some(text.clone());
            // GtkText can claim its own pointer sequence before an ancestor's bubble gesture sees it.
            // Observe the press in capture without consuming it: caret placement/selection stay native.
            let press = gtk::EventControllerLegacy::new();
            press.set_propagation_phase(gtk::PropagationPhase::Capture);
            let editable = text.downgrade();
            press.connect_event(move |_, event| {
                let activates = event.event_type() == gdk::EventType::TouchBegin
                    || event.event_type() == gdk::EventType::ButtonPress
                        && event.downcast_ref::<gdk::ButtonEvent>().is_some_and(|event| event.button() == gdk::BUTTON_PRIMARY);
                if activates && let Some(text) = editable.upgrade() {
                    crate::start::activate_taskbar_search(text.text().as_str());
                }
                glib::Propagation::Proceed
            });
            text.add_controller(press);
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(|_| {
                with(|bar| {
                    if let Some(text) = bar.search_text.borrow().clone() {
                        before_flyout();
                        crate::start::activate_taskbar_search(text.text().as_str());
                    }
                });
            });
            text.add_controller(focus);
            let clear_visible = clear.clone();
            text.connect_changed(move |text| {
                clear_visible.set_visible(!text.text().is_empty());
                with(|bar| {
                    if !bar.search_sync.get() {
                        crate::start::taskbar_search(text.text().as_str());
                    }
                });
            });
            let editable = text.clone();
            clear.connect_clicked(move |_| {
                editable.set_text("");
                editable.grab_focus();
            });
            let editable = text.clone();
            let editable_widget = text.clone().upcast::<gtk::Widget>();
            let clear_widget = clear.clone().upcast::<gtk::Widget>();
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            click.connect_pressed(move |gesture, _, x, y| {
                let hit = gesture.widget().and_then(|row| row.pick(x, y, gtk::PickFlags::DEFAULT));
                if hit.as_ref().is_some_and(|hit| *hit == editable_widget || hit.is_ancestor(&editable_widget))
                {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                if hit.is_some_and(|hit| hit == clear_widget || hit.is_ancestor(&clear_widget)) {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
                editable.grab_focus();
                crate::start::activate_taskbar_search(editable.text().as_str());
            });
            row.add_controller(click);
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            keys.connect_key_pressed(|_, key, _, state| {
                if crate::start::taskbar_search_key(key, state) { glib::Propagation::Stop }
                else { glib::Propagation::Proceed }
            });
            text.add_controller(keys);
            self.search_slot.append(&row);
            return;
        }
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
