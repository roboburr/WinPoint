//! Helpers of the taskbar: icons from the backend (`/icon?name=..`, tray pixmaps, our own assets) as textures,
//! the clock's date and time formats (core/format.js, nl-NL / en-GB), small popover menus and geometry.
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/* ------------------------------------------------------------------ icons */

type Waiter = Box<dyn FnOnce(Option<gdk::Texture>)>;

enum Entry {
    Ready(Option<gdk::Texture>),
    Pending(Vec<Waiter>),
}

thread_local! {
    static ICONS: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
}

/// Percent-encoding for a query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.to_lowercase().trim_end_matches(".desktop").to_owned()
}

/// The backend path of an icon (bridge.iconUrl): our Settings has its own asset, our apps a themed fallback,
/// other apps the theme's generic program icon (`themed` false: no fallback, e.g. tray icons).
pub fn icon_path(name: &str, px: i32, themed: bool) -> String {
    let n = norm(name);
    if [
        "win11shell-settings",
        "io.github.win11shell.settings",
        "preferences-system",
        "org.gnome.settings",
        "gnome-control-center",
        "systemsettings",
    ]
    .contains(&n.as_str())
    {
        return "/ui/assets/icons/settings.svg".into();
    }
    if name.starts_with("/ui/") {
        return name.to_owned();
    }
    let own = match n.as_str() {
        "win11shell-taskmgr" => Some("utilities-system-monitor"),
        "win11shell-explorer" => Some("system-file-manager"),
        "win11shell-alles" => Some("system-search"),
        _ => None,
    };
    let fallback = own.or(if themed && !n.starts_with("win11shell-") { Some("application-x-executable") } else { None });
    match fallback {
        Some(fb) => format!("/icon?name={}&size={px}&fallback={}", enc(name), enc(fb)),
        None => format!("/icon?name={}&size={px}", enc(name)),
    }
}

fn decode(bytes: &[u8], px: i32) -> Option<gdk::Texture> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    if head.contains("<svg") || head.trim_start().starts_with("<?xml") {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
        if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, px, px, true, gio::Cancellable::NONE) {
            #[allow(deprecated)]
            return Some(gdk::Texture::for_pixbuf(&pixbuf));
        }
    }
    crate::icons::texture(bytes)
}

/// Fetch a backend image path (cached); `done` runs on the main thread, None when it is missing.
pub fn fetch(shell: &Shell, path: String, px: i32, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    enum State {
        Ready(Option<gdk::Texture>),
        Pending,
        New,
    }
    let state = ICONS.with(|icons| match icons.borrow().get(&path) {
        Some(Entry::Ready(t)) => State::Ready(t.clone()),
        Some(Entry::Pending(_)) => State::Pending,
        None => State::New,
    });
    match state {
        State::Ready(t) => {
            done(t);
            return;
        }
        State::Pending => {
            ICONS.with(|icons| {
                if let Some(Entry::Pending(w)) = icons.borrow_mut().get_mut(&path) {
                    w.push(Box::new(done));
                }
            });
            return;
        }
        State::New => ICONS.with(|icons| {
            let mut icons = icons.borrow_mut();
            if icons.len() > 400 {
                icons.retain(|_, e| matches!(e, Entry::Pending(_)));
            }
            icons.insert(path.clone(), Entry::Pending(vec![Box::new(done)]));
        }),
    }
    let backend = shell.backend.clone();
    let key = path.clone();
    glib::spawn_future_local(async move {
        let bytes = gio::spawn_blocking(move || backend.get(&path)).await.ok().and_then(|r| r.ok());
        let texture = bytes.filter(|b| !b.is_empty()).and_then(|b| decode(&b, px));
        let waiters = ICONS.with(|icons| {
            let mut icons = icons.borrow_mut();
            match icons.insert(key.clone(), Entry::Ready(texture.clone())) {
                Some(Entry::Pending(w)) => w,
                _ => Vec::new(),
            }
        });
        for w in waiters {
            w(texture.clone());
        }
    });
}

/// Put an app / tray icon into `image` (`px` CSS pixels), loaded at twice that size for HiDPI. Until it is
/// there the image keeps what it shows; a missing icon becomes the theme icon or the "app" glyph.
pub fn set_icon(shell: &Shell, image: &gtk::Image, name: &str, px: i32, themed: bool) {
    image.set_pixel_size(px);
    let key = format!("{name}|{px}");
    unsafe {
        if image.data::<String>("w11-icon").is_some_and(|k| k.as_ref() == &key) {
            return;
        }
        image.set_data("w11-icon", key.clone());
    }
    if name.is_empty() {
        image.set_icon_name(Some("application-x-executable"));
        return;
    }
    let path = icon_path(name, (px * 2).max(32), themed);
    let weak = image.downgrade();
    let name = name.to_owned();
    let apply = move |texture: Option<gdk::Texture>| {
        let Some(image) = weak.upgrade() else { return };
        if unsafe { image.data::<String>("w11-icon").is_some_and(|k| k.as_ref() != &key) } {
            return;
        }
        match texture {
            Some(t) => image.set_paintable(Some(&t)),
            None if name.starts_with('/') => image.set_from_file(Some(&name)),
            None => {
                let theme = gtk::IconTheme::for_display(&image.display());
                let n = name.trim_end_matches(".desktop");
                if theme.has_icon(n) {
                    image.set_icon_name(Some(n));
                } else {
                    image.set_icon_name(Some("application-x-executable"));
                }
            }
        }
        image.set_pixel_size(px);
    };
    fetch(shell, path, (px * 2).max(32), apply);
}

/* ---------------------------------------------------------- date and time */

const NL_DAYS: [&str; 7] = ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"];
const EN_DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const NL_MONTHS: [&str; 12] = [
    "januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november",
    "december",
];
const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

/// time(ms, {seconds, h24}) of core/format.js: "14:05", "14:05:09"; 12 h: "2:05 p.m." (nl) / "2:05 pm" (en).
pub fn fmt_time(now: &glib::DateTime, lang: &str, seconds: bool, h24: bool) -> String {
    let (h, m, s) = (now.hour(), now.minute(), now.second());
    let secs = if seconds { format!(":{s:02}") } else { String::new() };
    if h24 {
        format!("{h:02}:{m:02}{secs}")
    } else {
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        let suffix = match (lang, h < 12) {
            ("en", true) => "am",
            ("en", false) => "pm",
            (_, true) => "a.m.",
            (_, false) => "p.m.",
        };
        format!("{h12}:{m:02}{secs} {suffix}")
    }
}

/// date(ms, 'short'): "25-10-2026" (nl-NL) / "25/10/2026" (en-GB).
pub fn fmt_date_short(now: &glib::DateTime, lang: &str) -> String {
    let sep = if lang == "en" { "/" } else { "-" };
    format!("{:02}{sep}{:02}{sep}{}", now.day_of_month(), now.month(), now.year())
}

/// date(ms, 'long'): "zaterdag 25 oktober 2026" / "Saturday 25 October 2026".
pub fn fmt_date_long(now: &glib::DateTime, lang: &str) -> String {
    let wd = (now.day_of_week().clamp(1, 7) - 1) as usize;
    let mo = (now.month().clamp(1, 12) - 1) as usize;
    if lang == "en" {
        format!("{} {} {} {}", EN_DAYS[wd], now.day_of_month(), EN_MONTHS[mo], now.year())
    } else {
        format!("{} {} {} {}", NL_DAYS[wd], now.day_of_month(), NL_MONTHS[mo], now.year())
    }
}

/* ------------------------------------------------------------------ menus */

/// One entry of a small popover menu; `None` action = a separator.
pub struct Item {
    pub label: String,
    pub action: Option<Rc<dyn Fn()>>,
}

pub fn item(label: String, action: impl Fn() + 'static) -> Item {
    Item { label, action: Some(Rc::new(action)) }
}

pub fn separator() -> Item {
    Item { label: String::new(), action: None }
}

/// A Win11-style context menu above the taskbar at `x` (strip coordinates), parented to `parent`.
pub fn popup_menu(parent: &gtk::Widget, x: i32, items: Vec<Item>) {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    for (i, it) in items.into_iter().enumerate() {
        match it.action {
            None => {
                if section.n_items() > 0 {
                    menu.append_section(None, &section);
                    section = gio::Menu::new();
                }
            }
            Some(f) => {
                let name = format!("m{i}");
                let action = gio::SimpleAction::new(&name, None);
                action.connect_activate(move |_, _| f());
                group.add_action(&action);
                section.append(Some(&it.label.replace('_', "__")), Some(&format!("tbm.{name}")));
            }
        }
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.add_css_class("w11-tb-menu");
    popover.insert_action_group("tbm", Some(&group));
    show_popover(parent, popover.upcast_ref(), x);
}

/// Parent a popover to the strip, point it at `x` on the strip's top edge, open it, and drop it when closed.
pub fn show_popover(parent: &gtk::Widget, popover: &gtk::Popover, x: i32) {
    popover.set_parent(parent);
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x, 0, 1, 1)));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

/// The centre x of `widget` in `target`'s coordinates (the strip or the monitor-sized panel window).
pub fn center_x(widget: &impl IsA<gtk::Widget>, target: &impl IsA<gtk::Widget>) -> i32 {
    widget
        .compute_bounds(target)
        .map(|b| (b.x() + b.width() / 2.0).round() as i32)
        .unwrap_or(0)
}

/// `widget`'s bounds in `target`'s coordinates.
pub fn bounds(widget: &impl IsA<gtk::Widget>, target: &impl IsA<gtk::Widget>) -> Option<gdk::Rectangle> {
    widget.compute_bounds(target).map(|b| {
        gdk::Rectangle::new(b.x().round() as i32, b.y().round() as i32, b.width().round() as i32, b.height().round() as i32)
    })
}

/// A point of `widget` in its window's coordinates (= the monitor's, the panel window covers it).
pub fn window_point(widget: &impl IsA<gtk::Widget>, x: f64, y: f64) -> (i32, i32) {
    let Some(root) = widget.root() else { return (x as i32, y as i32) };
    widget
        .compute_point(&root, &gtk::graphene::Point::new(x as f32, y as f32))
        .map(|p| (p.x().round() as i32, p.y().round() as i32))
        .unwrap_or((x as i32, y as i32))
}

/// Add a CSS class for `ms` milliseconds (keyframe animations: bounce, enter, pulse).
pub fn flash_class(widget: &impl IsA<gtk::Widget>, class: &'static str, ms: u64) {
    let w = widget.upcast_ref::<gtk::Widget>().clone();
    w.remove_css_class(class);
    w.add_css_class(class);
    let weak = w.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        if let Some(w) = weak.upgrade() {
            w.remove_css_class(class);
        }
    });
}

/// Toggle a CSS class.
pub fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// Open a page of our Settings app (shell.openApp settings {page}).
pub fn open_settings(shell: &Shell, page: &str) {
    shell.fire("shell.openApp", serde_json::json!({"app": "settings", "params": {"page": page}}));
}

/// Open one of our apps (shell.openApp {app}).
pub fn open_app(shell: &Shell, app: &str) {
    shell.fire("shell.openApp", serde_json::json!({"app": app, "params": {}}));
}

/// A place for a Fluent glyph that changes (network strength, volume, bell): the glyph is replaced only when
/// its name changes, whatever icons::glyph builds.
pub struct GlyphSlot {
    pub holder: gtk::Box,
    name: RefCell<String>,
}

impl GlyphSlot {
    pub fn new(name: &str, px: i32, class: &str) -> GlyphSlot {
        let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        holder.set_valign(gtk::Align::Center);
        holder.set_halign(gtk::Align::Center);
        if !class.is_empty() {
            holder.add_css_class(class);
        }
        let slot = GlyphSlot { holder, name: RefCell::new(String::new()) };
        slot.set(name, px);
        slot
    }

    pub fn set(&self, name: &str, px: i32) {
        if *self.name.borrow() == name {
            return;
        }
        *self.name.borrow_mut() = name.to_owned();
        while let Some(c) = self.holder.first_child() {
            self.holder.remove(&c);
        }
        self.holder.append(&crate::icons::glyph(name, px));
    }
}
