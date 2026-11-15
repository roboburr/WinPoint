//! The right end of the strip (taskbar.js): the system button (network, volume, battery -> quick settings),
//! the clock with the notification bell (-> notifications) and the show-desktop sliver.
use super::util;
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

pub struct System {
    shell: Rc<Shell>,
    strip: gtk::Widget,
    pub sys_btn: gtk::Button,
    net: util::GlyphSlot,
    vol: util::GlyphSlot,
    bat: gtk::DrawingArea,
    net_state: RefCell<Option<Value>>,
    audio_state: RefCell<Option<Value>>,
    battery_state: RefCell<Option<Value>>,
    bat_level: Cell<(f64, bool)>,
    vol_acc: Cell<f64>,
    pub clock_btn: gtk::Button,
    time: gtk::Label,
    date: gtk::Label,
    bell: gtk::Box,
    bell_img: util::GlyphSlot,
    count: gtk::Label,
    notifs: RefCell<Vec<Value>>,
    seen: RefCell<HashSet<String>>,
    clock_timer: RefCell<Option<glib::SourceId>>,
    pub desktop_btn: gtk::Button,
}

thread_local! {
    static SYS: RefCell<Option<Rc<System>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<System>) -> R) -> Option<R> {
    let s = SYS.with(|x| x.borrow().clone());
    s.map(|s| f(&s))
}

fn nid(v: &Value) -> String {
    match v.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn wifi_icon(strength: Option<f64>) -> &'static str {
    match strength {
        None => "wifi",
        Some(s) if s >= 67.0 => "wifi-3",
        Some(s) if s >= 34.0 => "wifi-2",
        Some(_) => "wifi-1",
    }
}

fn volume_icon(volume: f64, muted: bool) -> &'static str {
    if muted || volume <= 0.0 {
        "volume-mute"
    } else if volume < 34.0 {
        "volume-1"
    } else if volume < 67.0 {
        "volume-2"
    } else {
        "volume"
    }
}

pub fn build(shell: &Rc<Shell>, strip: &gtk::Widget) -> Rc<System> {
    // system button: network, volume, battery
    let sys_btn = gtk::Button::new();
    sys_btn.add_css_class("w11-tb-sys");
    sys_btn.add_css_class("w11-tb-hit");
    sys_btn.set_widget_name("tb-sys");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let net = util::GlyphSlot::new("globe", 16, "w11-tray-ico");
    let vol = util::GlyphSlot::new("volume", 16, "w11-tray-ico");
    let bat = gtk::DrawingArea::new();
    bat.add_css_class("w11-tray-ico");
    bat.add_css_class("w11-battery");
    bat.set_content_width(16);
    bat.set_content_height(16);
    bat.set_valign(gtk::Align::Center);
    bat.set_visible(false);
    row.append(&net.holder);
    row.append(&vol.holder);
    row.append(&bat);
    sys_btn.set_child(Some(&row));

    // clock: time over date, then the bell
    let clock_btn = gtk::Button::new();
    clock_btn.add_css_class("w11-tb-clock");
    clock_btn.add_css_class("w11-tb-hit");
    clock_btn.set_widget_name("tb-clock");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.add_css_class("w11-clock-text");
    text.set_valign(gtk::Align::Center);
    let time = gtk::Label::new(None);
    time.add_css_class("w11-clock-time");
    time.set_halign(gtk::Align::End);
    time.set_widget_name("tb-clock-time");
    let date = gtk::Label::new(None);
    date.add_css_class("w11-clock-date");
    date.set_halign(gtk::Align::End);
    date.set_widget_name("tb-clock-date");
    text.append(&time);
    text.append(&date);
    let bell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bell.add_css_class("w11-clock-bell");
    bell.set_valign(gtk::Align::Center);
    bell.set_halign(gtk::Align::Center);
    let bell_img = util::GlyphSlot::new("notification", 16, "");
    let count = gtk::Label::new(None);
    count.add_css_class("w11-clock-count");
    count.set_widget_name("tb-clock-count");
    count.set_visible(false);
    bell.append(&bell_img.holder);
    bell.append(&count);
    line.append(&text);
    line.append(&bell);
    clock_btn.set_child(Some(&line));

    // show desktop: a 12 px sliver with a 1 px line on hover
    let desktop_btn = gtk::Button::new();
    desktop_btn.add_css_class("w11-tb-desktop");
    desktop_btn.add_css_class("w11-tb-hit");
    desktop_btn.set_widget_name("tb-desktop");
    desktop_btn.set_vexpand(true);
    desktop_btn.set_valign(gtk::Align::Fill);
    let sliver = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sliver.add_css_class("w11-tb-desktop-line");
    sliver.set_halign(gtk::Align::Start);
    sliver.set_valign(gtk::Align::Fill);
    sliver.set_can_target(false);
    desktop_btn.set_child(Some(&sliver));

    let sys = Rc::new(System {
        shell: shell.clone(),
        strip: strip.clone(),
        sys_btn: sys_btn.clone(),
        net,
        vol,
        bat: bat.clone(),
        net_state: RefCell::new(None),
        audio_state: RefCell::new(None),
        battery_state: RefCell::new(None),
        bat_level: Cell::new((0.0, false)),
        vol_acc: Cell::new(0.0),
        clock_btn: clock_btn.clone(),
        time,
        date,
        bell,
        bell_img,
        count,
        notifs: RefCell::new(Vec::new()),
        seen: RefCell::new(HashSet::new()),
        clock_timer: RefCell::new(None),
        desktop_btn: desktop_btn.clone(),
    });
    SYS.with(|x| *x.borrow_mut() = Some(sys.clone()));

    bat.set_draw_func(|da, cr, w, h| {
        let Some((percent, charging)) = with(|s| s.bat_level.get()) else { return };
        draw_battery(da, cr, w, h, percent, charging);
    });

    sys_btn.connect_clicked(|_| {
        super::before_flyout();
        crate::core::shell().flyouts().toggle("quick");
    });
    clock_btn.connect_clicked(|_| {
        super::before_flyout();
        crate::core::shell().flyouts().toggle("notifications");
    });
    desktop_btn.connect_clicked(|_| {
        super::before_flyout();
        crate::core::shell().gnome.send("windows.showDesktop", json!({}));
    });

    // right click: the button's own menu
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    right.connect_pressed(|g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        with(|s| s.sys_menu(x, y));
    });
    sys_btn.add_controller(right);
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    right.connect_pressed(|g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        with(|s| s.clock_menu(x, y));
    });
    clock_btn.add_controller(right);

    // Win11 24H2: scrolling over the volume icon changes the volume
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        with(|s| s.scroll_volume(dy));
        glib::Propagation::Stop
    });
    sys.vol.holder.add_controller(scroll);

    // state
    for (method, slot) in [("system.network.get", 0), ("system.audio.get", 1), ("system.battery.get", 2)] {
        shell.call(method, json!({}), move |r| {
            with(|s| {
                let v = r.unwrap_or(Value::Null);
                match slot {
                    0 => *s.net_state.borrow_mut() = Some(v),
                    1 => *s.audio_state.borrow_mut() = Some(v),
                    _ => *s.battery_state.borrow_mut() = Some(v),
                }
                s.render_tray();
            });
        });
    }
    shell.on("system.network.changed", |_, d| {
        with(|s| {
            *s.net_state.borrow_mut() = Some(d.clone());
            s.render_tray();
        });
    });
    shell.on("system.audio.changed", |_, d| {
        with(|s| {
            *s.audio_state.borrow_mut() = Some(d.clone());
            s.render_tray();
        });
    });
    shell.on("system.battery.changed", |_, d| {
        with(|s| {
            *s.battery_state.borrow_mut() = Some(d.clone());
            s.render_tray();
        });
    });

    // notifications
    shell.call("notifications.list", json!({}), |r| {
        with(|s| {
            if let Ok(v) = r {
                *s.notifs.borrow_mut() = v["items"].as_array().cloned().unwrap_or_default();
            }
            s.render_bell();
        });
    });
    shell.on("notifications.new", |_, d| {
        let Some(n) = d.get("notification").filter(|n| n.is_object()).cloned() else { return };
        with(|s| {
            let id = nid(&n);
            {
                let mut list = s.notifs.borrow_mut();
                list.retain(|x| nid(x) != id);
                list.insert(0, n);
            }
            let open = s.shell.flyouts.borrow().as_ref().is_some_and(|f| f.is_open("notifications"));
            if open {
                s.seen.borrow_mut().insert(id);
            }
            s.render_bell();
            if s.shell.setting("notifications.dnd") != true {
                util::flash_class(&s.bell, "is-pulse", 420);
            }
        });
    });
    shell.on("notifications.closed", |_, d| {
        let id = match &d["id"] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return,
        };
        with(|s| {
            s.notifs.borrow_mut().retain(|x| nid(x) != id);
            s.render_bell();
        });
    });

    sys.update_texts();
    sys.render_tray();
    sys.tick();
    sys.render_bell();
    sys
}

/// Settings (`data`, core::touches) or the language changed: the texts and tooltips by the language, the clock by
/// time.* and taskbar.showSeconds, the bell by notifications.dnd, the battery's bolt by the theme.
pub fn settings_changed(data: &Value, lang_changed: bool) {
    let touches = |keys: &[&str]| lang_changed || crate::core::touches(data, keys);
    with(|s| {
        if lang_changed {
            s.update_texts();
            s.render_tray();
        }
        if touches(&["time", "taskbar.showSeconds"]) {
            s.tick();
        }
        if touches(&["notifications.dnd"]) {
            s.render_bell();
        }
        if crate::core::touches(data, &["theme"]) {
            s.bat.queue_draw();
        }
    });
}

/// The notifications flyout opened: everything in it counts as seen.
pub fn notifications_opened() {
    with(|s| {
        let ids: Vec<String> = s.notifs.borrow().iter().map(nid).collect();
        s.seen.borrow_mut().extend(ids);
        s.render_bell();
    });
}

impl System {
    fn update_texts(&self) {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        self.sys_btn.set_tooltip_text(Some(&t("Snelle instellingen", "Quick settings")));
        self.desktop_btn.set_tooltip_text(Some(&t("Bureaublad weergeven", "Show desktop")));
    }

    fn render_tray(&self) {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        // network
        let n = self.net_state.borrow().clone();
        let (mut glyph, mut tip) = ("globe".to_owned(), t("Netwerk", "Network"));
        if let Some(n) = n.filter(|n| n.is_object()) {
            let p = &n["primary"];
            let internet = |c: &Value| {
                if c.is_null() || c == "full" || c == "" { t("Internettoegang", "Internet access") } else { t("Geen internettoegang", "No internet access") }
            };
            if n["airplane"] == true {
                glyph = "airplane".into();
                tip = t("Vliegtuigmodus staat aan", "Flight mode is on");
            } else if p["type"] == "ethernet" {
                glyph = "ethernet".into();
                let name = p["name"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| t("Ethernet", "Ethernet"));
                tip = format!("{name}\n{}", internet(&n["connectivity"]));
            } else if p["type"] == "wifi" {
                glyph = wifi_icon(p["strength"].as_f64()).into();
                let name = p["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("Wi-Fi").to_owned();
                tip = format!("{name}\n{}", internet(&n["connectivity"]));
            } else {
                glyph = if n["wifiAvailable"] == true && n["wifiEnabled"] != false { "wifi-off".into() } else { "globe".into() };
                tip = t("Niet verbonden", "Not connected");
            }
        }
        self.net.set(&glyph, 16);
        self.net.holder.set_tooltip_text(Some(&tip));

        // volume
        let a = self.audio_state.borrow().clone().filter(|a| a.is_object());
        self.vol.holder.set_visible(a.as_ref().is_none_or(|a| a["available"] != false));
        let volume = a.as_ref().and_then(|a| a["volume"].as_f64()).unwrap_or(50.0).round();
        let muted = a.as_ref().is_some_and(|a| a["muted"] == true);
        self.vol.set(if a.is_some() { volume_icon(volume, muted) } else { "volume" }, 16);
        let vtip = match &a {
            Some(_) if muted => t("Geluid staat uit", "Sound is off"),
            Some(_) => t(&format!("Luidsprekers: {volume}%"), &format!("Speakers: {volume}%")),
            None => t("Geluidsinstellingen", "Sound settings"),
        };
        self.vol.holder.set_tooltip_text(Some(&vtip));

        // battery
        let b = self.battery_state.borrow().clone().filter(|b| b["present"] == true);
        self.bat.set_visible(b.is_some());
        if let Some(b) = b {
            let p = b["percent"].as_f64().unwrap_or(0.0).round();
            let on_ac = b["onAC"] == true;
            let charging = b["state"] == "charging" || (on_ac && p < 100.0);
            self.bat_level.set((p, charging));
            self.bat.queue_draw();
            let tip = if charging {
                t(&format!("{p}% beschikbaar (wordt opgeladen)"), &format!("{p}% available (charging)"))
            } else if on_ac {
                t(&format!("{p}% beschikbaar (aangesloten)"), &format!("{p}% available (plugged in)"))
            } else {
                t(&format!("Batterij: {p}%"), &format!("Battery: {p}%"))
            };
            self.bat.set_tooltip_text(Some(&tip));
        }
    }

    fn scroll_volume(&self, dy: f64) {
        let Some(a) = self.audio_state.borrow().clone().filter(|a| a.is_object() && a["available"] != false) else { return };
        let acc = self.vol_acc.get() + dy;
        let notches = acc.trunc();
        self.vol_acc.set(acc - notches);
        if notches == 0.0 {
            return;
        }
        let step = if notches < 0.0 { 2.0 } else { -2.0 } * notches.abs();
        let v = (a["volume"].as_f64().unwrap_or(0.0).round() + step).clamp(0.0, 100.0);
        let mut next = a.clone();
        next["volume"] = json!(v);
        if v > 0.0 {
            next["muted"] = json!(false);
        }
        *self.audio_state.borrow_mut() = Some(next);
        self.render_tray();
        self.shell.fire("system.audio.set", json!({"volume": v, "muted": false}));
    }

    fn sys_menu(&self, x: f64, y: f64) {
        let shell = self.shell.clone();
        let sx = self.strip_x(&self.sys_btn, x, y);
        let s1 = shell.clone();
        let s2 = shell.clone();
        let mut items = vec![
            util::item(shell.t("Netwerk- en internetinstellingen", "Network and Internet settings"), move || {
                util::open_settings(&s1, "network")
            }),
            util::item(shell.t("Geluidsinstellingen", "Sound settings"), move || util::open_settings(&s2, "sound")),
            util::separator(),
        ];
        items.extend(super::taskbar_items(&shell));
        util::popup_menu(&self.strip, sx, items);
    }

    fn clock_menu(&self, x: f64, y: f64) {
        let shell = self.shell.clone();
        let sx = self.strip_x(&self.clock_btn, x, y);
        let s1 = shell.clone();
        let s2 = shell.clone();
        let items = vec![
            util::item(shell.t("Datum en tijd aanpassen", "Adjust date and time"), move || util::open_settings(&s1, "datetime")),
            util::item(shell.t("Meldingsinstellingen", "Notification settings"), move || {
                util::open_settings(&s2, "notifications")
            }),
        ];
        util::popup_menu(&self.strip, sx, items);
    }

    fn strip_x(&self, widget: &gtk::Button, x: f64, y: f64) -> i32 {
        widget
            .compute_point(&self.strip, &gtk::graphene::Point::new(x as f32, y as f32))
            .map(|p| p.x().round() as i32)
            .unwrap_or(0)
    }

    /// The clock: HH:MM (seconds when asked), the short date, the long date and time as tooltip.
    fn tick(&self) {
        if let Some(id) = self.clock_timer.borrow_mut().take() {
            id.remove();
        }
        let Ok(now) = glib::DateTime::now_local() else { return };
        let settings = self.shell.settings();
        let lang = self.shell.lang();
        let h24 = settings["time"]["h24"] != false;
        let seconds = settings["time"]["showSeconds"] == true || settings["taskbar"]["showSeconds"] == true;
        let show_date = settings["time"]["showDate"] != false;
        self.time.set_text(&util::fmt_time(&now, lang, seconds, h24));
        self.date.set_text(&if show_date { util::fmt_date_short(&now, lang) } else { String::new() });
        self.date.set_visible(show_date);
        util::set_class(&self.clock_btn, "is-single", !show_date);
        let tip = format!("{}\n{}", util::fmt_date_long(&now, lang), util::fmt_time(&now, lang, true, h24));
        self.clock_btn.set_tooltip_text(Some(&tip));
        let step_ms: i64 = if seconds { 1000 } else { 60_000 };
        let into = (now.second() as i64 * 1000 + now.microsecond() as i64 / 1000) % step_ms;
        let wait = (step_ms - into + 20).max(20) as u64;
        *self.clock_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(wait), || {
            with(|s| {
                *s.clock_timer.borrow_mut() = None;
                s.tick();
            });
        }));
    }

    fn render_bell(&self) {
        let dnd = self.shell.setting("notifications.dnd") == true;
        let n = {
            let seen = self.seen.borrow();
            self.notifs.borrow().iter().filter(|x| !seen.contains(&nid(x))).count()
        };
        util::set_class(&self.bell, "is-dnd", dnd);
        util::set_class(&self.bell, "has-count", !dnd && n > 0);
        if dnd {
            self.bell_img.set("notification-off", 16);
            self.bell_img.holder.set_visible(true);
            self.count.set_visible(false);
        } else if n > 0 {
            self.bell_img.holder.set_visible(false);
            self.count.set_text(&if n > 9 { "9+".to_owned() } else { n.to_string() });
            self.count.set_visible(true);
        } else {
            self.bell_img.set("notification", 16);
            self.bell_img.holder.set_visible(true);
            self.count.set_visible(false);
        }
        let state = if dnd {
            self.shell.t("Niet storen staat aan", "Do not disturb is on")
        } else if n == 1 {
            self.shell.t("1 nieuwe melding", "1 new notification")
        } else if n > 1 {
            self.shell.t(&format!("{n} nieuwe meldingen"), &format!("{n} new notifications"))
        } else {
            self.shell.t("Geen nieuwe meldingen", "No new notifications")
        };
        self.bell.set_tooltip_text(Some(&state));
    }
}

/// Battery glyph with the charge level drawn in (Win11 style), 20 x 20 design units scaled to the area.
fn draw_battery(da: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, percent: f64, charging: bool) {
    let color = da.color();
    let scale = (w.min(h) as f64) / 20.0;
    cr.scale(scale, scale);
    let rounded = |cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64| {
        cr.new_sub_path();
        cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
    };
    let (r, g, b, a) = (color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
    cr.set_source_rgba(r, g, b, a);
    cr.set_line_width(1.0);
    rounded(cr, 2.0, 5.5, 14.0, 9.0, 2.0);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.move_to(18.0, 8.5);
    cr.line_to(18.0, 11.5);
    let _ = cr.stroke();
    let fill = (10.5 * percent.clamp(0.0, 100.0) / 100.0 * 10.0).round() / 10.0;
    let fill = fill.max(1.0);
    if percent <= 10.0 && !charging {
        cr.set_source_rgba(0xe8 as f64 / 255.0, 0x11 as f64 / 255.0, 0x23 as f64 / 255.0, 1.0);
    } else {
        cr.set_source_rgba(r, g, b, a);
    }
    rounded(cr, 3.75, 7.25, fill, 5.5, 1.0_f64.min(fill / 2.0));
    let _ = cr.fill();
    if charging {
        cr.move_to(9.8, 6.2);
        cr.line_to(7.6, 10.3);
        cr.line_to(10.2, 10.3);
        cr.line_to(9.2, 13.8);
        cr.line_to(12.1, 9.4);
        cr.line_to(9.5, 9.4);
        cr.line_to(10.5, 6.2);
        cr.close_path();
        let dark = crate::core::shell().dark();
        if dark {
            cr.set_source_rgba(0x1c as f64 / 255.0, 0x1c as f64 / 255.0, 0x1c as f64 / 255.0, 1.0);
        } else {
            cr.set_source_rgba(0xee as f64 / 255.0, 0xee as f64 / 255.0, 0xee as f64 / 255.0, 1.0);
        }
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_width(0.8);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        let _ = cr.stroke();
    }
}
