//! Flyout `notifications` (the clock, Win+N): `git show 418ea594:vendor/ui/js/shell/notifcenter.js` and toasts.js
//! ported. Two stacked cards like Windows 11: the notifications grouped per app (Niet storen, Alles wissen, the app's
//! buttons, a close button per notification and per group, "Geen nieuwe meldingen") and the calendar
//! (calendar.rs) under it. The list comes from notifications.list and stays live through notifications.new and
//! notifications.closed (the backend's stream and the extension's signal; a notification that arrives on both
//! counts once).
//!
//! A notification that arrives while the centre is closed shows as a toast bottom right above the clock for
//! 6 s (not with Niet storen unless critical, not with notifications.showToasts off, at most 3; hovering keeps
//! it). Toasts live in the panel window's overlay, take the pointer through the input region and never the
//! keyboard. The shell is the extension's notification consumer: it renews the banner lease
//! (notifications.consumerReady, every 2 s, as the WebKit panel did) so GNOME's own banners stay away while
//! ours show, and acknowledges GNOME's notifications (notifications.delivered).
use super::calendar::{self, Calendar, icon_button, set_label};
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

const NAME: &str = "notifications";
const TOAST_MAX: usize = 3;
const TOAST_MS: u64 = 6000;
const WIDTH: i32 = 360;
const TOAST_WIDTH: i32 = 364;

thread_local! {
    static CENTER: RefCell<Option<Rc<Center>>> = const { RefCell::new(None) };
}

/// Texts as in notifcenter.js / toasts.js.
fn tr(key: &str) -> String {
    let (nl, en) = match key {
        "title" => ("Meldingen", "Notifications"),
        "clearAll" => ("Alles wissen", "Clear all"),
        "empty" => ("Geen nieuwe meldingen", "No new notifications"),
        "unsupported" => (
            "Meldingen van apps verschijnen hier zodra je de Win11 Shell-sessie gebruikt.",
            "Notifications from apps appear here once you use the Win11 Shell session.",
        ),
        "dnd" => ("Niet storen", "Do not disturb"),
        "dndOn" => ("Niet storen staat aan", "Do not disturb is on"),
        "dndOff" => ("Niet storen staat uit", "Do not disturb is off"),
        "dndNote" => (
            "Niet storen staat aan. Je ziet geen pop-ups; nieuwe meldingen komen hier binnen.",
            "Do not disturb is on. You won\u{2019}t see pop-ups; new notifications arrive here.",
        ),
        "dismiss" => ("Melding sluiten", "Dismiss notification"),
        "dismissGroup" => ("Alle meldingen van {app} wissen", "Clear all notifications from {app}"),
        "notification" => ("Melding", "Notification"),
        "yesterday" => ("gisteren", "yesterday"),
        "close" => ("Sluiten", "Close"),
        "shell" => ("Win11 Shell", "Win11 Shell"),
        _ => (key, key),
    };
    crate::core::shell().t(nl, en)
}

/// The flyout's registration, the toasts and the consumer lease.
pub fn register(shell: &Rc<Shell>) {
    let center = Center::new();
    CENTER.with(|c| *c.borrow_mut() = Some(center.clone()));
    shell.flyouts().register(Rc::new(NotificationsFlyout { center: center.clone() }));

    let weak = Rc::downgrade(&center);
    shell.on("notifications.new", move |_, data| {
        if let Some(c) = weak.upgrade() {
            c.arrived(data);
        }
    });
    let weak = Rc::downgrade(&center);
    shell.on("notifications.closed", move |_, data| {
        if let Some(c) = weak.upgrade() {
            c.closed_remote(&data["id"]);
        }
    });
    let weak = Rc::downgrade(&center);
    shell.on("settings.changed", move |_, _| {
        if let Some(c) = weak.upgrade() {
            c.settings_changed();
        }
    });
    // Banner lease: while it holds, GNOME leaves the banners to us (extension/notificationsBridge.js).
    let weak = Rc::downgrade(&center);
    let beat = move || {
        if let Some(c) = weak.upgrade() {
            c.heartbeat();
        }
    };
    beat();
    glib::timeout_add_seconds_local(2, move || {
        beat();
        glib::ControlFlow::Continue
    });
    center.refresh();
}

/// The number of notifications (for a badge on the taskbar).
pub fn count() -> usize {
    CENTER.with(|c| c.borrow().as_ref().map(|c| c.items.borrow().len()).unwrap_or(0))
}

/// Open the flyout with the keyboard on the calendar (notifcenter.js command `calendar`).
pub fn open_calendar() {
    let shell = crate::core::shell();
    shell.flyouts().open(NAME);
    if let Some(c) = CENTER.with(|c| c.borrow().clone()) {
        glib::idle_add_local_once(move || c.cal.focus());
    }
}

/// A local toast from a shell module (toasts.js showToast): not stored in the notification centre.
pub fn toast(title: &str, body: &str) {
    if let Some(c) = CENTER.with(|c| c.borrow().clone()) {
        let n = json!({"appName": tr("shell"), "appIcon": "win11shell", "summary": title, "body": body});
        c.toasts.present(&c, &n, None);
    }
}

struct NotificationsFlyout {
    center: Rc<Center>,
}

impl Flyout for NotificationsFlyout {
    fn name(&self) -> &'static str {
        NAME
    }
    fn widget(&self) -> gtk::Widget {
        self.center.root.clone().upcast()
    }
    fn placement(&self) -> Placement {
        Placement::BottomRight
    }
    fn opened(&self) {
        self.center.opened();
    }
    fn closed(&self) {
        self.center.closed();
    }
}

// ---------------------------------------------------------------------------------------------------------
// Data helpers

fn id_key(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => n.as_u64().map(|u| u.to_string()).or_else(|| n.as_f64().map(|f| format!("{f}"))),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn s<'a>(n: &'a Value, key: &str) -> &'a str {
    n.get(key).and_then(Value::as_str).unwrap_or("")
}

fn time_of(n: &Value) -> f64 {
    let t = &n["time"];
    t.as_f64().or_else(|| t.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0)
}

fn is_critical(n: &Value) -> bool {
    let u = &n["urgency"];
    u.as_i64() == Some(2) || u.as_str().is_some_and(|s| s == "2" || s.eq_ignore_ascii_case("critical"))
}

fn has_default(n: &Value) -> bool {
    n["hasDefault"] == true || actions_of(n, true).iter().any(|(k, _)| k == "default")
}

/// (key, label) of the notification's buttons; without `all` the default action and empty labels drop out.
fn actions_of(n: &Value, all: bool) -> Vec<(String, String)> {
    let Some(list) = n["actions"].as_array() else { return Vec::new() };
    let out = list.iter().filter_map(|a| {
        let key = a["key"].as_str().map(str::to_owned).or_else(|| a["key"].as_i64().map(|k| k.to_string()))?;
        let label = a["label"].as_str().unwrap_or("").to_owned();
        Some((key, label))
    });
    if all { out.collect() } else { out.filter(|(k, l)| k != "default" && !l.is_empty()).take(3).collect() }
}

fn app_key(n: &Value) -> String {
    let name = s(n, "appName");
    let icon = s(n, "appIcon");
    (if !name.is_empty() { name } else if !icon.is_empty() { icon } else { "?" }).to_lowercase()
}

fn is_shell_name(name: &str) -> bool {
    let l = name.to_lowercase();
    l == "win11 shell" || l == "win11shell"
}

/// Notification texts may carry simple markup (<b>, <i>, <a>, <br>): plain text only.
fn plain(text: &str) -> String {
    if !text.contains('<') && !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => {
                let tag = rest[i + 1..i + j].trim().to_lowercase();
                if tag.starts_with("br") {
                    out.push('\n');
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn local_time(ms: f64) -> Option<glib::DateTime> {
    if ms <= 0.0 {
        return None;
    }
    glib::DateTime::from_unix_local((ms / 1000.0) as i64).ok()
}

fn clock(dt: &glib::DateTime, shell: &Shell) -> String {
    let (h, m) = (dt.hour(), dt.minute());
    if shell.setting("time.h24") != false {
        format!("{h:02}:{m:02}")
    } else {
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        let suffix = if shell.lang() == "en" {
            if h < 12 { "am" } else { "pm" }
        } else if h < 12 {
            "a.m."
        } else {
            "p.m."
        };
        format!("{h12}:{m:02} {suffix}")
    }
}

/// Windows 11's time stamp: "14:05" today, "gisteren", otherwise "za 24 okt" / "Sat 24 Oct".
fn stamp(ms: f64) -> String {
    let Some(dt) = local_time(ms) else { return String::new() };
    let shell = crate::core::shell();
    let en = shell.lang() == "en";
    let day: calendar::Ymd = (dt.year(), dt.month().clamp(1, 12) as u32, dt.day_of_month().clamp(1, 31) as u32);
    let diff = calendar::day_number(calendar::today()) - calendar::day_number(day);
    if diff <= 0 {
        clock(&dt, &shell)
    } else if diff == 1 {
        tr("yesterday")
    } else {
        format!("{} {} {}", calendar::day_short(calendar::weekday(day), en), day.2, calendar::month_short(day.1, en))
    }
}

/// "zaterdag 25 oktober 2026 om 14:05" / "Saturday 25 October 2026 at 14:05" (the stamp's tooltip).
fn full_stamp(ms: f64) -> Option<String> {
    let dt = local_time(ms)?;
    let shell = crate::core::shell();
    let en = shell.lang() == "en";
    let day: calendar::Ymd = (dt.year(), dt.month().clamp(1, 12) as u32, dt.day_of_month().clamp(1, 31) as u32);
    Some(format!("{} {} {}", calendar::full_date(day, en), if en { "at" } else { "om" }, clock(&dt, &shell)))
}

fn has_icon(name: &str) -> bool {
    gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name))
}

/// The Windows logo for the shell's own notifications (startLogo of icons.js).
fn start_logo(px: i32) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(1);
    grid.set_column_spacing(1);
    let side = ((px - 1) / 2).max(1);
    for i in 0..4 {
        let square = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        square.set_size_request(side, side);
        square.add_css_class("fo-startlogo");
        grid.attach(&square, i % 2, i / 2, 1, 1);
    }
    grid.set_halign(gtk::Align::Center);
    grid.set_valign(gtk::Align::Center);
    grid.set_size_request(px, px);
    grid.upcast()
}

/// The app's icon: a theme name, a path, our own logo, or the notification bell.
fn app_icon(n: &Value, px: i32) -> gtk::Widget {
    let icon = s(n, "appIcon");
    let app = s(n, "appName");
    if icon == "win11shell" || (icon.is_empty() && is_shell_name(app)) {
        return start_logo(px);
    }
    let name = if icon.is_empty() { app.to_lowercase() } else { icon.to_owned() };
    let image = if name.starts_with('/') && std::path::Path::new(&name).exists() {
        crate::icons::app_icon(&name, px)
    } else if !name.is_empty() && !name.starts_with('/') && !name.contains(':') && has_icon(name.trim_end_matches(".desktop")) {
        crate::icons::app_icon(&name, px)
    } else if has_icon("preferences-system-notifications") {
        crate::icons::app_icon("preferences-system-notifications", px)
    } else {
        crate::icons::glyph("notification", px)
    };
    image.set_valign(gtk::Align::Center);
    image.upcast()
}

#[derive(Clone)]
enum Img {
    Texture(gdk::Texture),
    Icon(String),
}

/// A notification image: data: URLs, file:// URLs or absolute paths, or a theme icon name (never remote).
fn load_image(src: &str) -> Option<Img> {
    if src.is_empty() {
        return None;
    }
    if let Some(rest) = src.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        if !meta.ends_with(";base64") {
            return None;
        }
        let bytes = glib::base64_decode(data);
        return crate::icons::texture(&bytes).map(Img::Texture);
    }
    let path = if src.starts_with("file://") {
        gio::File::for_uri(src).path()?
    } else if src.starts_with('/') {
        std::path::PathBuf::from(src)
    } else if src.contains("://") {
        return None;
    } else if src.chars().all(|c| c.is_alphanumeric() || "_.+-".contains(c)) {
        return has_icon(src).then(|| Img::Icon(src.to_owned()));
    } else {
        return None;
    };
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX) > 16 * 1024 * 1024 {
        return None;
    }
    gdk::Texture::from_filename(&path).ok().map(Img::Texture)
}

/// The texture scaled and cropped to exactly w x h (CSS object-fit: cover), as a paintable of that size.
fn cover(texture: &gdk::Texture, w: f32, h: f32) -> Option<gdk::Paintable> {
    let (tw, th) = (texture.width().max(1) as f32, texture.height().max(1) as f32);
    let scale = (w / tw).max(h / th);
    let (sw, sh) = (tw * scale, th * scale);
    let snapshot = gtk::Snapshot::new();
    snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, w, h));
    snapshot.append_texture(texture, &graphene::Rect::new((w - sw) / 2.0, (h - sh) / 2.0, sw, sh));
    snapshot.pop();
    snapshot.to_paintable(Some(&graphene::Size::new(w, h)))
}

/// Windows 11 shows avatars and small pictures as a 48 px logo left of the text and wide pictures as a hero
/// image under it: (logo, hero).
fn image_widgets(img: Option<Img>, hero_width: i32, class: &str) -> (Option<gtk::Widget>, Option<gtk::Widget>) {
    match img {
        None => (None, None),
        Some(Img::Icon(name)) => {
            let image = gtk::Image::from_icon_name(&name);
            image.set_pixel_size(48);
            image.add_css_class(&format!("{class}-logo"));
            image.add_css_class("is-round");
            image.set_valign(gtk::Align::Start);
            (Some(image.upcast()), None)
        }
        Some(Img::Texture(texture)) => {
            let (w, h) = (texture.width().max(1) as f32, texture.height().max(1) as f32);
            let hero = w >= 160.0 && w / h >= 1.3;
            if hero {
                let width = hero_width as f32;
                let height = (width * h / w).min(180.0).max(24.0);
                let Some(p) = cover(&texture, width, height) else { return (None, None) };
                let picture = gtk::Picture::for_paintable(&p);
                picture.set_can_shrink(true);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.add_css_class(&format!("{class}-hero"));
                picture.set_overflow(gtk::Overflow::Hidden);
                (None, Some(picture.upcast()))
            } else {
                let Some(p) = cover(&texture, 48.0, 48.0) else { return (None, None) };
                let picture = gtk::Picture::for_paintable(&p);
                picture.set_can_shrink(false);
                picture.add_css_class(&format!("{class}-logo"));
                if (w / h - 1.0).abs() < 0.15 {
                    picture.add_css_class("is-round");
                }
                picture.set_overflow(gtk::Overflow::Hidden);
                picture.set_valign(gtk::Align::Start);
                (Some(picture.upcast()), None)
            }
        }
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    for c in classes {
        l.add_css_class(c);
    }
    l.set_xalign(0.0);
    l
}

/// How many lines of a notification's body show: `lines`, or 12 for one GNOME shows expanded (its app's "Inhoud
/// weergeven in banners", force-expanded, or a critical one: the bridge's `expanded`).
fn body_lines(n: &Value, lines: i32) -> i32 {
    if n["expanded"] == true { 12 } else { lines }
}

/// What the extension's bridge needs of WinPoint's notification settings with every call (amendment 4: no sound
/// while notifications.sounds or notifications.showToasts is off), so it reads no settings file.
fn bridge_settings(shell: &Shell) -> (bool, bool) {
    bridge_flags(&shell.setting("notifications.sounds"), &shell.setting("notifications.showToasts"))
}

/// `sounds` and `toasts` for the bridge from notifications.sounds and notifications.showToasts: on unless false (a
/// settings file without them has both on).
fn bridge_flags(sounds: &Value, toasts: &Value) -> (bool, bool) {
    (*sounds != false, *toasts != false)
}

fn wrapped(text: &str, classes: &[&str], lines: i32) -> gtk::Label {
    let l = label(text, classes);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_lines(lines);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(30);
    l.set_hexpand(true);
    l
}

/// Whether (x, y) of `root` lands on a button inside it (the card's own click then stays out).
fn on_button(root: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut w = root.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = w {
        if &widget == root {
            return false;
        }
        if widget.is::<gtk::Button>() {
            return true;
        }
        w = widget.parent();
    }
    false
}

fn clear_box(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// Slide out to the right and fade (CSS), then fold the height away, then `done`.
fn collapse_out(revealer: &gtk::Revealer, class: &'static str, done: impl FnOnce() + 'static) {
    revealer.add_css_class(class);
    let r = revealer.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(170), move || {
        let Some(r) = r.upgrade() else { return done() };
        r.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        r.set_transition_duration(167);
        r.set_reveal_child(false);
        glib::timeout_add_local_once(Duration::from_millis(190), done);
    });
}

fn shown_revealer(child: &impl IsA<gtk::Widget>) -> gtk::Revealer {
    let r = gtk::Revealer::new();
    r.set_transition_duration(0);
    r.set_reveal_child(true);
    r.set_child(Some(child));
    r.add_css_class("nc-anim");
    r
}

fn invoke(n: &Value, key: &str) {
    crate::core::shell().call("notifications.invokeAction", json!({"id": n["id"], "key": key}), |r| {
        if let Err(e) = r {
            eprintln!("win11-shell: notifications.invokeAction: {e}");
        }
    });
}

// ---------------------------------------------------------------------------------------------------------
// The notification centre

struct Card {
    id: String,
    group: String,
    revealer: gtk::Revealer,
    card: gtk::Widget,
}

struct Group {
    key: String,
    name: String,
    sample: Value,
    items: Vec<Value>,
    latest: f64,
}

struct Center {
    items: RefCell<Vec<Value>>,
    supported: Cell<bool>,
    dnd: Cell<bool>,
    lang: Cell<&'static str>,
    root: gtk::Box,
    card: gtk::Box,
    header: gtk::Box,
    title: gtk::Label,
    dnd_btn: gtk::Button,
    clear_btn: gtk::Button,
    note: gtk::Box,
    note_label: gtk::Label,
    scroller: gtk::ScrolledWindow,
    list: gtk::Box,
    cal: Rc<Calendar>,
    cards: RefCell<Vec<Card>>,
    groups: RefCell<Vec<(String, gtk::Revealer)>>,
    stamps: RefCell<Vec<(gtk::Label, f64)>>,
    tick: RefCell<Option<glib::SourceId>>,
    images: RefCell<HashMap<String, Option<Img>>>,
    recent: RefCell<Vec<(String, String, Instant)>>,
    toasts: Rc<Toasts>,
    me: RefCell<Weak<Center>>,
}

impl Center {
    fn new() -> Rc<Center> {
        let shell = crate::core::shell();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("fo-nc-wrap");
        root.set_size_request(WIDTH, -1);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("nc-surface");
        card.add_css_class("fo-nc");
        card.set_overflow(gtk::Overflow::Hidden);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.add_css_class("fo-nc-header");
        let title = label("", &["nc-body", "nc-strong"]);
        title.set_hexpand(true);
        let dnd_btn = icon_button("notification-off", 32, 16);
        dnd_btn.add_css_class("fo-nc-dnd");
        let clear_btn = gtk::Button::with_label("");
        clear_btn.add_css_class("nc-btn");
        clear_btn.add_css_class("nc-btn-subtle");
        clear_btn.add_css_class("nc-btn-small");
        clear_btn.add_css_class("fo-nc-clear");
        clear_btn.set_valign(gtk::Align::Center);
        header.append(&title);
        header.append(&dnd_btn);
        header.append(&clear_btn);

        let note = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        note.add_css_class("fo-nc-note");
        let note_icon = crate::icons::glyph("notification-off", 12);
        note_icon.set_valign(gtk::Align::Start);
        note_icon.set_margin_top(2);
        let note_label = wrapped("", &["nc-caption"], 4);
        note.append(&note_icon);
        note.append(&note_label);
        note.set_visible(false);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
        list.add_css_class("fo-nc-list");
        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("fo-nc-scroll");
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(600);
        scroller.set_child(Some(&list));

        card.append(&header);
        card.append(&note);
        card.append(&scroller);

        let cal = Calendar::new();
        root.append(&card);
        root.append(&cal.widget());

        let center = Rc::new(Center {
            items: RefCell::new(Vec::new()),
            supported: Cell::new(true),
            dnd: Cell::new(shell.setting("notifications.dnd") == true),
            lang: Cell::new(shell.lang()),
            root,
            card,
            header,
            title,
            dnd_btn,
            clear_btn,
            note,
            note_label,
            scroller,
            list,
            cal,
            cards: RefCell::new(Vec::new()),
            groups: RefCell::new(Vec::new()),
            stamps: RefCell::new(Vec::new()),
            tick: RefCell::new(None),
            images: RefCell::new(HashMap::new()),
            recent: RefCell::new(Vec::new()),
            toasts: Toasts::new(),
            me: RefCell::new(Weak::new()),
        });
        *center.me.borrow_mut() = Rc::downgrade(&center);

        let weak = Rc::downgrade(&center);
        center.dnd_btn.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.toggle_dnd();
            }
        });
        let weak = Rc::downgrade(&center);
        center.clear_btn.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.clear_all();
            }
        });
        let weak = Rc::downgrade(&center);
        center.cal.on_toggle(move |_| {
            if let Some(c) = weak.upgrade() {
                c.fit();
            }
        });
        // ArrowDown/ArrowUp from the flyout itself go to the first notification (or the calendar).
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&center);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(c) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !matches!(key, gdk::Key::Down | gdk::Key::Up) || c.focused_card().is_some() || c.cal.has_focus() {
                return glib::Propagation::Proceed;
            }
            let first = c.cards.borrow().first().map(|card| card.card.clone());
            match first {
                Some(card) => card.grab_focus(),
                None => c.cal.header().grab_focus(),
            };
            glib::Propagation::Stop
        });
        center.root.add_controller(keys);

        center.relabel();
        center.render_list(None);
        center
    }

    fn relabel(&self) {
        self.title.set_text(&tr("title"));
        self.clear_btn.set_label(&tr("clearAll"));
        self.note_label.set_text(&tr("dndNote"));
        set_label(&self.card, &tr("title"));
        set_label(&self.cal.widget(), &crate::core::shell().t("Agenda", "Calendar"));
    }

    fn is_open(&self) -> bool {
        crate::core::shell().flyouts.borrow().as_ref().is_some_and(|f| f.is_open(NAME))
    }

    fn heartbeat(&self) {
        let shell = crate::core::shell();
        let (sounds, toasts) = bridge_settings(&shell);
        shell.gnome.send(
            "notifications.consumerReady",
            json!({"pid": std::process::id(), "ready": true, "dnd": self.dnd.get(), "sounds": sounds, "toasts": toasts}),
        );
    }

    fn refresh(&self) {
        let weak = self.me.borrow().clone();
        crate::core::shell().call("notifications.list", json!({}), move |result| {
            let Some(c) = weak.upgrade() else { return };
            match result {
                Ok(r) => {
                    c.supported.set(r["supported"] != false);
                    *c.items.borrow_mut() = r["items"].as_array().cloned().unwrap_or_default();
                }
                Err(_) => {
                    c.supported.set(false);
                    c.items.borrow_mut().clear();
                }
            }
            if c.is_open() {
                c.render_list(None);
            } else {
                c.render_header();
            }
        });
    }

    /// The same notification over the backend stream and the extension's signal counts once.
    fn seen(&self, id: &str, n: &Value) -> bool {
        let text = n.to_string();
        let now = Instant::now();
        let mut recent = self.recent.borrow_mut();
        recent.retain(|(_, _, t)| now.duration_since(*t) < Duration::from_secs(3));
        if recent.iter().any(|(i, s, _)| i == id && *s == text) {
            return true;
        }
        recent.push((id.to_owned(), text, now));
        false
    }

    fn arrived(&self, data: &Value) {
        let n = if data["notification"].is_object() { data["notification"].clone() } else { data.clone() };
        let Some(id) = id_key(&n["id"]) else { return };
        if self.seen(&id, &n) {
            return;
        }
        let shell = crate::core::shell();
        {
            let mut items = self.items.borrow_mut();
            items.retain(|x| id_key(&x["id"]).as_deref() != Some(id.as_str()));
            items.push(n.clone());
        }
        if s(&n, "source") == "gnome" {
            let (sounds, toasts) = bridge_settings(&shell);
            shell.gnome.send(
                "notifications.delivered",
                json!({"id": n["id"], "pid": std::process::id(), "sounds": sounds, "toasts": toasts}),
            );
        }
        if self.is_open() {
            self.render_list(Some(&id));
            return;
        }
        self.render_header();
        // toasts.js: not for silent GNOME notifications, not with toasts off, not with Niet storen (unless
        // critical), not while the centre is open.
        if s(&n, "source") == "gnome" && n["silent"] == true {
            return;
        }
        if shell.setting("notifications.showToasts") == false {
            return;
        }
        if self.dnd.get() && !is_critical(&n) {
            return;
        }
        if let Some(me) = self.me.borrow().upgrade() {
            self.toasts.present(&me, &n, Some(id));
        }
    }

    fn closed_remote(&self, id: &Value) {
        let Some(id) = id_key(id) else { return };
        self.toasts.close_id(&id);
        let known = self.items.borrow().iter().any(|x| id_key(&x["id"]).as_deref() == Some(id.as_str()));
        if !known {
            return;
        }
        self.items.borrow_mut().retain(|x| id_key(&x["id"]).as_deref() != Some(id.as_str()));
        self.remove_card(&id);
    }

    fn settings_changed(&self) {
        let shell = crate::core::shell();
        let dnd = shell.setting("notifications.dnd") == true;
        let dnd_changed = dnd != self.dnd.get();
        self.dnd.set(dnd);
        if dnd_changed {
            self.heartbeat();
        }
        let lang = shell.lang();
        if lang != self.lang.get() {
            self.lang.set(lang);
            self.relabel();
            self.render_list(None);
        } else {
            self.render_header();
        }
        self.cal.render();
    }

    fn toggle_dnd(&self) {
        let dnd = !self.dnd.get();
        self.dnd.set(dnd);
        crate::core::shell().set_setting("notifications.dnd", json!(dnd));
        self.heartbeat();
        self.render_header();
        self.fit();
    }

    fn render_header(&self) {
        let dnd = self.dnd.get();
        if dnd { self.dnd_btn.add_css_class("is-on") } else { self.dnd_btn.remove_css_class("is-on") }
        let text = if dnd { tr("dndOn") } else { tr("dndOff") };
        self.dnd_btn.set_tooltip_text(Some(&text));
        set_label(&self.dnd_btn, &text);
        self.dnd_btn.update_state(&[gtk::accessible::State::Pressed(if dnd {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        })]);
        self.note.set_visible(dnd);
        self.clear_btn.set_visible(!self.items.borrow().is_empty());
    }

    /// Room for the list: the monitor minus the strip, the margins and the calendar.
    fn fit(&self) {
        let height = self.root.root().and_downcast::<gtk::Window>().map(|w| w.height()).filter(|h| *h > 0);
        let height = height.unwrap_or(1080);
        let header = self.header.measure(gtk::Orientation::Vertical, WIDTH).1;
        let note = if self.note.is_visible() { self.note.measure(gtk::Orientation::Vertical, WIDTH).1 + 8 } else { 0 };
        let room = height - crate::panel::BAR - 24 - 12 - self.cal.height() - header - note - 2;
        self.scroller.set_max_content_height(room.max(96));
    }

    fn groups(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::new();
        for n in self.items.borrow().iter() {
            let key = app_key(n);
            let t = time_of(n);
            match out.iter_mut().find(|g| g.key == key) {
                Some(g) => {
                    g.items.push(n.clone());
                    g.latest = g.latest.max(t);
                }
                None => {
                    let name = s(n, "appName");
                    out.push(Group {
                        key,
                        name: if name.is_empty() { tr("notification") } else { plain(name) },
                        sample: n.clone(),
                        items: vec![n.clone()],
                        latest: t,
                    });
                }
            }
        }
        for g in out.iter_mut() {
            g.items.sort_by(|a, b| time_of(b).total_cmp(&time_of(a)));
        }
        out.sort_by(|a, b| b.latest.total_cmp(&a.latest));
        out
    }

    fn focused_card(&self) -> Option<String> {
        let focus = self.root.root().and_then(|r| r.focus())?;
        self.cards
            .borrow()
            .iter()
            .find(|c| focus == c.card || focus.is_ancestor(&c.card))
            .map(|c| c.id.clone())
    }

    fn image(&self, src: &str) -> Option<Img> {
        if src.is_empty() {
            return None;
        }
        let mut cache = self.images.borrow_mut();
        if let Some(hit) = cache.get(src) {
            return hit.clone();
        }
        if cache.len() > 64 {
            cache.clear();
        }
        let img = load_image(src);
        cache.insert(src.to_owned(), img.clone());
        img
    }

    fn render_list(&self, added: Option<&str>) {
        self.render_header();
        let focus_id = self.focused_card();
        clear_box(&self.list);
        self.cards.borrow_mut().clear();
        self.groups.borrow_mut().clear();
        self.stamps.borrow_mut().clear();
        if self.items.borrow().is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
            empty.add_css_class("fo-nc-empty");
            let text = label(&tr("empty"), &["nc-body", "nc-text-2"]);
            text.set_xalign(0.5);
            text.set_halign(gtk::Align::Center);
            empty.append(&text);
            if !self.supported.get() {
                let hint = wrapped(&tr("unsupported"), &["nc-caption", "nc-text-3"], 4);
                hint.set_justify(gtk::Justification::Center);
                hint.set_xalign(0.5);
                hint.set_margin_top(6);
                hint.set_margin_start(34);
                hint.set_margin_end(34);
                empty.append(&hint);
            }
            self.list.append(&empty);
            return;
        }
        for g in self.groups() {
            let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
            group.add_css_class("fo-nc-group");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            head.add_css_class("fo-nc-ghead");
            head.append(&app_icon(&g.sample, 16));
            let name = label(&g.name, &["nc-caption", "fo-nc-gname"]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            head.append(&name);
            let close = icon_button("close", 24, 10);
            close.add_css_class("fo-nc-gclose");
            let tip = tr("dismissGroup").replace("{app}", &g.name);
            close.set_tooltip_text(Some(&tip));
            set_label(&close, &tip);
            let weak = self.me.borrow().clone();
            let key = g.key.clone();
            close.connect_clicked(move |_| {
                if let Some(c) = weak.upgrade() {
                    c.dismiss_group(&key);
                }
            });
            head.append(&close);
            group.append(&head);
            set_label(&group, &g.name);
            for n in &g.items {
                let Some(id) = id_key(&n["id"]) else { continue };
                let card = self.build_card(n, &id);
                let revealer = shown_revealer(&card);
                group.append(&revealer);
                self.cards.borrow_mut().push(Card { id, group: g.key.clone(), revealer, card });
            }
            let revealer = shown_revealer(&group);
            self.list.append(&revealer);
            self.groups.borrow_mut().push((g.key.clone(), revealer));
        }
        let cards = self.cards.borrow();
        if let Some(added) = added
            && let Some(card) = cards.iter().find(|c| c.id == added)
        {
            card.revealer.add_css_class("is-entering");
            let r = card.revealer.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(30), move || {
                if let Some(r) = r.upgrade() {
                    r.remove_css_class("is-entering");
                }
            });
        }
        if let Some(focus_id) = focus_id
            && let Some(card) = cards.iter().find(|c| c.id == focus_id)
        {
            card.card.grab_focus();
        }
    }

    fn build_card(&self, n: &Value, id: &str) -> gtk::Widget {
        let clickable = has_default(n);
        let app = s(n, "appName");
        let app = if app.is_empty() { tr("notification") } else { plain(app) };
        let summary = plain(s(n, "summary"));
        let body = plain(s(n, "body"));

        let card = gtk::Overlay::new();
        card.add_css_class("fo-notif");
        if clickable {
            card.add_css_class("is-clickable");
        }
        if is_critical(n) {
            card.add_css_class("is-critical");
        }
        card.set_focusable(true);
        let aria: Vec<&str> = [app.as_str(), summary.as_str(), body.as_str()].into_iter().filter(|t| !t.is_empty()).collect();
        set_label(&card, &aria.join(", "));

        let inner = gtk::Box::new(gtk::Orientation::Vertical, 2);
        inner.add_css_class("fo-notif-inner");
        let main = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let (logo, hero) = image_widgets(self.image(s(n, "image")), WIDTH - 24 - 2 - 27, "fo-notif");
        if let Some(logo) = logo {
            main.append(&logo);
        }
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title_text = if summary.is_empty() { app.clone() } else { summary.clone() };
        let title = wrapped(&title_text, &["nc-body", "nc-strong", "fo-notif-title"], 2);
        top.append(&title);
        let t = time_of(n);
        let time = label(&stamp(t), &["nc-caption", "nc-text-3", "fo-notif-time"]);
        time.set_valign(gtk::Align::Start);
        time.set_margin_top(2);
        if let Some(full) = full_stamp(t) {
            time.set_tooltip_text(Some(&full));
        }
        top.append(&time);
        self.stamps.borrow_mut().push((time, t));
        text.append(&top);
        if !body.is_empty() {
            text.append(&wrapped(&body, &["nc-body", "nc-text-2", "fo-notif-body"], body_lines(n, 4)));
        }
        main.append(&text);
        inner.append(&main);
        if let Some(hero) = hero {
            hero.set_margin_top(10);
            inner.append(&hero);
        }
        let actions = actions_of(n, false);
        if !actions.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_homogeneous(true);
            row.set_margin_top(10);
            for (key, text) in actions {
                let button = gtk::Button::with_label(&plain(&text));
                button.add_css_class("nc-btn");
                if let Some(l) = button.child().and_downcast::<gtk::Label>() {
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                let n = n.clone();
                button.connect_clicked(move |_| {
                    invoke(&n, &key);
                    crate::core::shell().flyouts().close();
                });
                row.append(&button);
            }
            inner.append(&row);
        }
        card.set_child(Some(&inner));

        let close = icon_button("close", 24, 10);
        close.add_css_class("fo-notif-close");
        close.set_halign(gtk::Align::End);
        close.set_valign(gtk::Align::Start);
        close.set_margin_top(7);
        close.set_margin_end(7);
        let tip = tr("dismiss");
        close.set_tooltip_text(Some(&tip));
        set_label(&close, &tip);
        let weak = self.me.borrow().clone();
        let id_owned = id.to_owned();
        close.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.dismiss(&id_owned);
            }
        });
        card.add_overlay(&close);

        let widget: gtk::Widget = card.clone().upcast();
        if clickable {
            let click = gtk::GestureClick::new();
            let w = widget.downgrade();
            let n2 = n.clone();
            click.connect_released(move |_, _, x, y| {
                let Some(w) = w.upgrade() else { return };
                if on_button(&w, x, y) {
                    return;
                }
                invoke(&n2, "default");
                crate::core::shell().flyouts().close();
            });
            card.add_controller(click);
        }
        let keys = gtk::EventControllerKey::new();
        let weak = self.me.borrow().clone();
        let id_owned = id.to_owned();
        let n2 = n.clone();
        let w = widget.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            let (Some(c), Some(w)) = (weak.upgrade(), w.upgrade()) else { return glib::Propagation::Proceed };
            let focus = w.root().and_then(|r| r.focus());
            if focus.as_ref() != Some(&w) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Delete | gdk::Key::KP_Delete => {
                    let next = c.neighbour(&id_owned, 1).or_else(|| c.neighbour(&id_owned, -1));
                    c.dismiss(&id_owned);
                    if let Some(next) = next {
                        next.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space if clickable => {
                    invoke(&n2, "default");
                    crate::core::shell().flyouts().close();
                    glib::Propagation::Stop
                }
                gdk::Key::Down | gdk::Key::Up => {
                    let step = if key == gdk::Key::Down { 1 } else { -1 };
                    if let Some(next) = c.neighbour(&id_owned, step) {
                        next.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        card.add_controller(keys);
        widget
    }

    fn neighbour(&self, id: &str, step: i32) -> Option<gtk::Widget> {
        let cards = self.cards.borrow();
        let i = cards.iter().position(|c| c.id == id)? as i32;
        let j = (i + step).clamp(0, cards.len() as i32 - 1);
        if j == i { None } else { cards.get(j as usize).map(|c| c.card.clone()) }
    }

    fn remove_card(&self, id: &str) {
        let (target, is_group, group_key) = {
            let mut cards = self.cards.borrow_mut();
            let Some(pos) = cards.iter().position(|c| c.id == id) else {
                drop(cards);
                if self.cards.borrow().is_empty() {
                    self.render_list(None);
                } else {
                    self.render_header();
                }
                return;
            };
            let card = cards.remove(pos);
            let last = !cards.iter().any(|c| c.group == card.group);
            let group = if last {
                self.groups.borrow().iter().find(|(k, _)| *k == card.group).map(|(_, r)| r.clone())
            } else {
                None
            };
            match group {
                Some(g) => (g, true, card.group),
                None => (card.revealer, false, card.group),
            }
        };
        if is_group {
            self.groups.borrow_mut().retain(|(k, _)| *k != group_key);
        }
        let weak = self.me.borrow().clone();
        let t = target.clone();
        collapse_out(&target, "is-leaving", move || {
            if let Some(parent) = t.parent().and_downcast::<gtk::Box>() {
                parent.remove(&t);
            }
            if let Some(c) = weak.upgrade() {
                if c.items.borrow().is_empty() {
                    c.render_list(None);
                } else {
                    c.render_header();
                }
            }
        });
    }

    fn dismiss(&self, id: &str) {
        let original = self
            .items
            .borrow()
            .iter()
            .find(|x| id_key(&x["id"]).as_deref() == Some(id))
            .map(|x| x["id"].clone());
        self.items.borrow_mut().retain(|x| id_key(&x["id"]).as_deref() != Some(id));
        self.remove_card(id);
        if let Some(original) = original {
            crate::core::shell().call("notifications.dismiss", json!({"id": original}), |r| {
                if let Err(e) = r {
                    eprintln!("win11-shell: notifications.dismiss: {e}");
                }
            });
        }
    }

    /// The close button of a group header: the whole group slides out at once.
    fn dismiss_group(&self, key: &str) {
        let ids: Vec<Value> = self.items.borrow().iter().filter(|n| app_key(n) == key).map(|n| n["id"].clone()).collect();
        self.items.borrow_mut().retain(|n| app_key(n) != key);
        self.cards.borrow_mut().retain(|c| c.group != key);
        let group = self.groups.borrow().iter().find(|(k, _)| k == key).map(|(_, r)| r.clone());
        self.groups.borrow_mut().retain(|(k, _)| k != key);
        let weak = self.me.borrow().clone();
        let after = move || {
            if let Some(c) = weak.upgrade() {
                if c.items.borrow().is_empty() {
                    c.render_list(None);
                } else {
                    c.render_header();
                }
            }
        };
        match group {
            Some(g) => {
                let t = g.clone();
                collapse_out(&g, "is-leaving-far", move || {
                    if let Some(parent) = t.parent().and_downcast::<gtk::Box>() {
                        parent.remove(&t);
                    }
                    after();
                });
            }
            None => after(),
        }
        let shell = crate::core::shell();
        for id in ids {
            shell.call("notifications.dismiss", json!({"id": id}), |r| {
                if let Err(e) = r {
                    eprintln!("win11-shell: notifications.dismiss: {e}");
                }
            });
        }
    }

    fn clear_all(&self) {
        self.items.borrow_mut().clear();
        self.clear_btn.set_visible(false);
        let groups: Vec<gtk::Revealer> = self.groups.borrow().iter().map(|(_, r)| r.clone()).collect();
        for (i, g) in groups.iter().enumerate() {
            let g = g.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(i as u64 * 40), move || {
                if let Some(g) = g.upgrade() {
                    g.add_css_class("is-leaving-far");
                }
            });
        }
        let weak = self.me.borrow().clone();
        glib::timeout_add_local_once(Duration::from_millis(220 + groups.len() as u64 * 40), move || {
            if let Some(c) = weak.upgrade() {
                c.render_list(None);
            }
        });
        let weak = self.me.borrow().clone();
        crate::core::shell().call("notifications.clear", json!({}), move |r| {
            if let Err(e) = r {
                eprintln!("win11-shell: notifications.clear: {e}");
                if let Some(c) = weak.upgrade() {
                    c.refresh();
                }
            }
        });
    }

    fn opened(&self) {
        self.cal.reset();
        self.render_list(None);
        self.refresh();
        self.toasts.close_all();
        self.fit();
        let weak = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(c) = weak.upgrade() {
                c.fit();
            }
        });
        if let Some(old) = self.tick.borrow_mut().take() {
            old.remove();
        }
        // Keep the time stamps and today's date fresh while open.
        let weak = self.me.borrow().clone();
        let id = glib::timeout_add_seconds_local(60, move || {
            let Some(c) = weak.upgrade() else { return glib::ControlFlow::Break };
            for (label, t) in c.stamps.borrow().iter() {
                label.set_text(&stamp(*t));
            }
            if !c.cal.has_focus() {
                c.cal.render();
            }
            glib::ControlFlow::Continue
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn closed(&self) {
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Toasts

struct Toast {
    id: Option<String>,
    outer: gtk::Revealer,
    timer: RefCell<Option<glib::SourceId>>,
    hover: Cell<bool>,
    closed: Cell<bool>,
    critical: bool,
}

struct Toasts {
    stack: gtk::Box,
    window: RefCell<Option<gtk::Window>>,
    live: RefCell<Vec<Rc<Toast>>>,
    me: RefCell<Weak<Toasts>>,
}

impl Toasts {
    fn new() -> Rc<Toasts> {
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 12);
        stack.add_css_class("fo-toasts");
        stack.set_halign(gtk::Align::End);
        stack.set_valign(gtk::Align::End);
        stack.set_margin_end(12);
        stack.set_margin_bottom(crate::panel::BAR + 12);
        let toasts = Rc::new(Toasts {
            stack,
            window: RefCell::new(None),
            live: RefCell::new(Vec::new()),
            me: RefCell::new(Weak::new()),
        });
        *toasts.me.borrow_mut() = Rc::downgrade(&toasts);
        toasts.install();
        toasts
    }

    /// Into the panel window's overlay, above the flyouts' dismiss layer.
    fn install(&self) -> bool {
        if self.window.borrow().is_some() {
            return true;
        }
        let shell = crate::core::shell();
        for w in shell.app.windows() {
            if !w.has_css_class("w11-panel-window") {
                continue;
            }
            if let Some(overlay) = w.child().and_downcast::<gtk::Overlay>() {
                overlay.add_overlay(&self.stack);
                *self.window.borrow_mut() = Some(w);
                return true;
            }
        }
        false
    }

    /// The pointer reaches the toasts: their area joins the panel window's input region (panel/mod.rs), whatever
    /// flyout is open.
    fn update_input(&self) {
        let Some(host) = crate::core::shell().flyouts.borrow().clone() else { return };
        let Some(win) = self.window.borrow().clone() else { return };
        let area = self.stack.first_child().and_then(|_| self.stack.compute_bounds(&win)).map(|b| {
            gtk::cairo::RectangleInt::new(
                b.x().floor() as i32,
                b.y().floor() as i32,
                b.width().ceil() as i32 + 1,
                b.height().ceil() as i32 + 1,
            )
        });
        host.set_extra_input(area);
    }

    fn update_input_soon(&self) {
        for ms in [40u64, 400] {
            let weak = self.me.borrow().clone();
            glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                if let Some(t) = weak.upgrade() {
                    t.update_input();
                }
            });
        }
    }

    fn close_id(&self, id: &str) {
        let found = self.live.borrow().iter().find(|t| t.id.as_deref() == Some(id)).cloned();
        if let Some(t) = found {
            self.close(&t, false);
        }
    }

    fn close_all(&self) {
        let all: Vec<_> = self.live.borrow().clone();
        for t in all {
            self.close(&t, false);
        }
    }

    fn arm(&self, toast: &Rc<Toast>, ms: u64) {
        if let Some(old) = toast.timer.borrow_mut().take() {
            old.remove();
        }
        if toast.critical || toast.closed.get() {
            return;
        }
        let weak_t = Rc::downgrade(toast);
        let weak_s = self.me.borrow().clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            let (Some(t), Some(s)) = (weak_t.upgrade(), weak_s.upgrade()) else { return };
            // The source is gone once it ran: forget its id so nobody removes it again.
            drop(t.timer.borrow_mut().take());
            if !t.hover.get() {
                s.close(&t, false);
            }
        });
        *toast.timer.borrow_mut() = Some(id);
    }

    fn disarm(&self, toast: &Toast) {
        if let Some(old) = toast.timer.borrow_mut().take() {
            old.remove();
        }
    }

    fn close(&self, toast: &Rc<Toast>, replace: bool) {
        if toast.closed.replace(true) {
            return;
        }
        self.disarm(toast);
        self.live.borrow_mut().retain(|t| !Rc::ptr_eq(t, toast));
        let outer = toast.outer.clone();
        if replace {
            self.stack.remove(&outer);
            self.update_input_soon();
            return;
        }
        let weak = self.me.borrow().clone();
        let o = outer.clone();
        collapse_out(&outer, "is-leaving", move || {
            if let Some(s) = weak.upgrade() {
                if o.parent().is_some() {
                    s.stack.remove(&o);
                }
                s.update_input();
            }
        });
    }

    fn present(&self, center: &Rc<Center>, n: &Value, id: Option<String>) {
        if !self.install() {
            return;
        }
        // A replacement keeps its notification ID: exactly one visible card.
        if let Some(id) = &id {
            let old = self.live.borrow().iter().find(|t| t.id.as_deref() == Some(id.as_str())).cloned();
            if let Some(old) = old {
                self.close(&old, true);
            }
        }
        // Too many on screen: the oldest leaves (it is still in the notification centre).
        loop {
            let oldest = {
                let live = self.live.borrow();
                if live.len() < TOAST_MAX { None } else { live.first().cloned() }
            };
            match oldest {
                Some(t) => self.close(&t, false),
                None => break,
            }
        }

        let critical = is_critical(n);
        let clickable = id.is_some() && has_default(n);
        let app_raw = s(n, "appName");
        let app = if app_raw.is_empty() { tr("notification") } else { plain(app_raw) };
        let title = plain(s(n, "summary"));
        let body = plain(s(n, "body"));

        let toast = gtk::Box::new(gtk::Orientation::Vertical, 0);
        toast.add_css_class("nc-surface");
        toast.add_css_class("fo-toast");
        if clickable {
            toast.add_css_class("is-clickable");
        }
        if critical {
            toast.add_css_class("is-critical");
        }
        toast.set_size_request(TOAST_WIDTH, -1);
        let aria: Vec<&str> = [app.as_str(), title.as_str(), body.as_str()].into_iter().filter(|t| !t.is_empty()).collect();
        set_label(&toast, &aria.join(", "));

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.add_css_class("fo-toast-head");
        head.append(&app_icon(n, 16));
        let app_label = label(&app, &["nc-caption", "nc-text-2"]);
        app_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        app_label.set_hexpand(true);
        head.append(&app_label);
        let close = icon_button("close", 28, 10);
        close.add_css_class("fo-toast-close");
        set_label(&close, &tr("close"));
        head.append(&close);
        toast.append(&head);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.add_css_class("fo-toast-content");
        let (logo, hero) = image_widgets(center.image(s(n, "image")), TOAST_WIDTH - 2 - 32, "fo-toast");
        if let Some(logo) = logo {
            content.append(&logo);
        }
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        if !title.is_empty() {
            text.append(&wrapped(&title, &["nc-body", "nc-strong"], 2));
        }
        if !body.is_empty() {
            text.append(&wrapped(&body, &["nc-body", "nc-text-2"], body_lines(n, 3)));
        }
        content.append(&text);
        toast.append(&content);
        if let Some(hero) = hero {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.add_css_class("fo-toast-hero-slot");
            slot.append(&hero);
            toast.append(&slot);
        }

        let outer = shown_revealer(&toast);
        outer.add_css_class("fo-toast-slot");
        let entry = Rc::new(Toast {
            id: id.clone(),
            outer: outer.clone(),
            timer: RefCell::new(None),
            hover: Cell::new(false),
            closed: Cell::new(false),
            critical,
        });

        let actions = if id.is_some() { actions_of(n, false) } else { Vec::new() };
        if !actions.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("fo-toast-actions");
            row.set_homogeneous(true);
            for (key, text) in actions {
                let button = gtk::Button::with_label(&plain(&text));
                button.add_css_class("nc-btn");
                if let Some(l) = button.child().and_downcast::<gtk::Label>() {
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                let n = n.clone();
                let weak_t = Rc::downgrade(&entry);
                let weak_s = self.me.borrow().clone();
                button.connect_clicked(move |_| {
                    invoke(&n, &key);
                    if let (Some(t), Some(s)) = (weak_t.upgrade(), weak_s.upgrade()) {
                        s.close(&t, false);
                    }
                });
                row.append(&button);
            }
            toast.append(&row);
        }

        let weak_t = Rc::downgrade(&entry);
        let weak_s = self.me.borrow().clone();
        close.connect_clicked(move |_| {
            if let (Some(t), Some(s)) = (weak_t.upgrade(), weak_s.upgrade()) {
                s.close(&t, false);
            }
        });
        let click = gtk::GestureClick::new();
        let weak_t = Rc::downgrade(&entry);
        let weak_s = self.me.borrow().clone();
        let tw = toast.clone().upcast::<gtk::Widget>().downgrade();
        let n2 = n.clone();
        click.connect_released(move |_, _, x, y| {
            let (Some(t), Some(s), Some(w)) = (weak_t.upgrade(), weak_s.upgrade(), tw.upgrade()) else { return };
            if on_button(&w, x, y) || !clickable {
                return;
            }
            invoke(&n2, "default");
            s.close(&t, false);
        });
        toast.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let weak_t = Rc::downgrade(&entry);
        let weak_s = self.me.borrow().clone();
        motion.connect_enter(move |_, _, _| {
            if let (Some(t), Some(s)) = (weak_t.upgrade(), weak_s.upgrade()) {
                t.hover.set(true);
                s.disarm(&t);
            }
        });
        let weak_t = Rc::downgrade(&entry);
        let weak_s = self.me.borrow().clone();
        motion.connect_leave(move |_| {
            if let (Some(t), Some(s)) = (weak_t.upgrade(), weak_s.upgrade()) {
                t.hover.set(false);
                s.arm(&t, 3000);
            }
        });
        toast.add_controller(motion);

        // Frosted glass slides in from the right; the HTML's FLIP becomes the stack's own layout.
        outer.add_css_class("is-arriving");
        self.stack.append(&outer);
        self.live.borrow_mut().push(entry.clone());
        let o = outer.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            if let Some(o) = o.upgrade() {
                o.remove_css_class("is-arriving");
            }
        });
        self.arm(&entry, TOAST_MS);
        self.update_input_soon();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expanded_notification_shows_its_whole_body() {
        // GNOME's "Inhoud weergeven in banners" (force-expanded) or a critical one: the bridge's expanded
        let expanded = json!({"id": 1, "body": "x", "expanded": true});
        assert_eq!(body_lines(&expanded, 3), 12, "the toast");
        assert_eq!(body_lines(&expanded, 4), 12, "the centre's card");
        for n in [json!({"id": 1, "body": "x", "expanded": false}), json!({"id": 1, "body": "x"}), json!({"expanded": "yes"})] {
            assert_eq!(body_lines(&n, 3), 3);
            assert_eq!(body_lines(&n, 4), 4);
        }
    }

    #[test]
    fn the_bridge_gets_the_sound_and_toast_settings_with_every_call() {
        assert_eq!(bridge_flags(&Value::Null, &Value::Null), (true, true), "a settings file without them");
        assert_eq!(bridge_flags(&json!(true), &json!(true)), (true, true));
        assert_eq!(bridge_flags(&json!(false), &json!(true)), (false, true));
        assert_eq!(bridge_flags(&json!(true), &json!(false)), (true, false));
        assert_eq!(bridge_flags(&json!(false), &json!(false)), (false, false));
    }
}
