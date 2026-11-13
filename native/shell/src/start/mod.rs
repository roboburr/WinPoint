//! Native Start (startmenu.js and search.js in GTK): a 642 x 726 window with the application ID
//! io.github.win11shell.Start, so the extension positions it above the taskbar and blurs it (nativeBackdrop
//! kind "start", the page's 90 % tint). The process owns io.github.win11shell.Start on the session bus with the
//! actions the extension activates over org.gtk.Actions (start-toggle, start-dismiss, start-quit) and
//! org.freedesktop.Application.Activate (panel.toggleStart of a resident Start).
//!
//! Contents: the search box (typing turns Start into the search view in place), Vastgemaakt (6 x 3 pages, dots,
//! wheel paging, drag to reorder), Alle apps (letter headers and the letter grid), Aanbevolen (recent files and new
//! apps, Meer) and the footer with the user menu, folder shortcuts and the power menu. It hides on focus loss, Esc
//! and every launch.
pub mod data;
pub mod menu;
pub mod search;

use crate::core::Shell;
use data::{App, FileItem, RecItem, RecKind, User};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use menu::{Anchor, Item, MenuLayer};
use search::clear;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

const PATH: &str = "/io/github/win11shell/Start";
const COLS: usize = 6;
const TILE_W: i32 = 96;
const TILE_H: i32 = 84;
const LETTERS: &str = "&#ABCDEFGHIJKLMNOPQRSTUVWXYZ";

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Home,
    All,
    Rec,
    Search,
}

impl View {
    fn name(self) -> &'static str {
        match self {
            View::Home => "home",
            View::All => "all",
            View::Rec => "rec",
            View::Search => "search",
        }
    }
}

type Texts = Vec<Box<dyn Fn(&Shell)>>;

pub struct Start {
    shell: Rc<Shell>,
    window: gtk::Window,
    panel: gtk::Box,
    top: gtk::Box,
    pub menu: Rc<MenuLayer>,
    text: gtk::Text,
    clear_btn: gtk::Button,
    stack: gtk::Stack,
    view: Cell<View>,
    texts: Texts,
    lang: RefCell<String>,
    // home
    pin_grid: gtk::Grid,
    dots: gtk::Box,
    pinned_area: gtk::Overlay,
    page: Cell<usize>,
    rec_grid: gtk::Grid,
    more_btn: gtk::Button,
    // all apps
    all_list: gtk::Box,
    all_scroll: gtk::ScrolledWindow,
    letters: gtk::Grid,
    all_heads: RefCell<Vec<(char, gtk::Button)>>,
    all_dirty: Cell<bool>,
    // recommended "Meer"
    recmore_grid: gtk::Grid,
    search: Rc<search::Search>,
    // footer
    footer: gtk::Box,
    user_btn: gtk::Button,
    foot_right: gtk::Box,
    power_btn: gtk::Button,
    // data
    apps: RefCell<Vec<App>>,
    browser: RefCell<Option<String>>,
    rec_items: RefCell<Vec<RecItem>>,
    user: RefCell<User>,
    apps_seq: Cell<u64>,
    apps_timer: RefCell<Option<glib::SourceId>>,
    // lifecycle
    open: Cell<bool>,
    epoch: Cell<u64>,
    focused: Cell<bool>,
    focus_hide_at: Cell<Option<Instant>>,
    glass_known: Cell<bool>,
    wheel_at: Cell<Option<Instant>>,
}

thread_local! {
    static START: RefCell<Option<Rc<Start>>> = const { RefCell::new(None) };
    static BUS: RefCell<Option<gio::OwnerId>> = const { RefCell::new(None) };
}

pub(crate) fn get() -> Option<Rc<Start>> {
    START.with(|s| s.borrow().clone())
}

pub fn install(shell: &Rc<Shell>) {
    let start = Start::build(shell);
    START.with(|s| *s.borrow_mut() = Some(start.clone()));
    start.connect();
    export_bus();
    crate::theme::refresh(shell);
    start.apply_surface();
    start.relabel();
    start.load_apps();
    start.load_user();
    start.configure_backdrop();
    start.render_footer();

    shell.on("apps.changed", |_, _| {
        if let Some(s) = get() {
            s.load_apps_soon();
        }
    });
    shell.on("settings.changed", |_, _| {
        if let Some(s) = get() {
            s.on_settings();
        }
    });
    shell.on("nativeBackdrop.changed", |_, data| {
        if let Some(s) = get() {
            s.apply_backdrop(data);
        }
    });
    // Resident: the surface exists before the first open, so it opens at once.
    WidgetExt::realize(&start.window);
}

/// The taskbar's Start button (and the extension's panel.toggleStart through Activate): open or close Start.
pub fn toggle() {
    match get() {
        Some(s) => s.toggle(),
        None => crate::core::shell().gnome.send("panel.toggleStart", json!({})),
    }
}

/// The taskbar's search box: open Start in its search view, with this text typed.
pub fn open_search(query: &str) {
    match get() {
        Some(s) => s.show(Some(query)),
        None => toggle(),
    }
}

/// Close Start (a launch, a result, a menu action).
pub(crate) fn dismiss() {
    if let Some(s) = get() {
        s.dismiss();
    }
}

pub(crate) fn start_pins() -> Vec<String> {
    get().map(|s| s.pins()).unwrap_or_default()
}

pub(crate) fn pin_start(id: &str) {
    if let Some(s) = get() {
        let mut l: Vec<String> = s.pins().into_iter().filter(|x| x != id).collect();
        l.push(id.to_owned());
        data::set_start_pins(&s.shell, l);
    }
}

pub(crate) fn unpin_start(id: &str) {
    if let Some(s) = get() {
        data::set_start_pins(&s.shell, s.pins().into_iter().filter(|x| x != id).collect());
    }
}

/* ===================================================================== bus */

fn export_bus() {
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn()| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        group.add_action(&a);
    };
    add("start-toggle", toggle);
    add("start-dismiss", || {
        if let Some(s) = get() {
            s.dismiss_after_focus_loss();
        }
    });
    // The extension stops Start when it is disabled: the menu goes, the shell (and the taskbar's process) stays.
    add("start-quit", || {
        if let Some(s) = get() {
            s.hide_now();
        }
    });
    let id = gio::bus_own_name(
        gio::BusType::Session,
        crate::START_ID,
        gio::BusNameOwnerFlags::REPLACE,
        move |conn, _| {
            if let Err(e) = conn.export_action_group(PATH, &group) {
                eprintln!("win11-shell: Start actions: {e}");
            }
            register_application(&conn, &group);
        },
        |_, _| {},
        |_, name| eprintln!("win11-shell: lost the bus name {name}"),
    );
    BUS.with(|b| *b.borrow_mut() = Some(id));
}

const APPLICATION_XML: &str = r#"<node>
  <interface name="org.freedesktop.Application">
    <method name="Activate"><arg type="a{sv}" name="platform_data" direction="in"/></method>
    <method name="Open"><arg type="as" name="uris" direction="in"/><arg type="s" name="hint" direction="in"/><arg type="a{sv}" name="platform_data" direction="in"/></method>
    <method name="ActivateAction"><arg type="s" name="action_name" direction="in"/><arg type="av" name="parameter" direction="in"/><arg type="a{sv}" name="platform_data" direction="in"/></method>
  </interface>
</node>"#;

/// org.freedesktop.Application at /io/github/win11shell/Start: the extension's _activateStart (Activate) toggles.
fn register_application(conn: &gio::DBusConnection, group: &gio::SimpleActionGroup) {
    let Ok(node) = gio::DBusNodeInfo::for_xml(APPLICATION_XML) else { return };
    let Some(iface) = node.lookup_interface("org.freedesktop.Application") else { return };
    let group = group.clone();
    let result = conn
        .register_object(PATH, &iface)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let token = |i: usize| {
                let pd = params.try_child_value(i)?;
                glib::VariantDict::new(Some(&pd))
                    .lookup_value("activation-token", Some(glib::VariantTy::STRING))
                    .and_then(|v| v.str().map(str::to_owned))
            };
            match method {
                "Activate" | "Open" => {
                    if let Some(s) = get() {
                        if let Some(t) = token(if method == "Open" { 2 } else { 0 }) {
                            s.window.set_startup_id(&t);
                        }
                        s.toggle();
                    }
                }
                "ActivateAction" => {
                    if let Some(name) = params.try_child_value(0).and_then(|v| v.str().map(str::to_owned))
                        && let Some(action) = group.lookup_action(&name)
                    {
                        action.activate(None);
                    }
                }
                _ => {}
            }
            invocation.return_value(None);
        })
        .build();
    match result {
        Ok(id) => std::mem::forget(id),
        Err(e) => eprintln!("win11-shell: Start application object: {e}"),
    }
}

/* ================================================================ helpers */

fn tlabel(texts: &mut Texts, nl: &'static str, en: &'static str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_xalign(0.0);
    let c = l.clone();
    texts.push(Box::new(move |s| c.set_label(&s.t(nl, en))));
    l
}

/// A pill button ("Alle apps >", "< Terug").
fn pill(texts: &mut Texts, nl: &'static str, en: &'static str, glyph: &str, after: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-pillbtn");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let l = tlabel(texts, nl, en, "");
    if !after {
        b.add_css_class("is-back");
        row.append(&crate::icons::glyph(glyph, 12));
    }
    row.append(&l);
    if after {
        row.append(&crate::icons::glyph(glyph, 12));
    }
    b.set_child(Some(&row));
    b
}

fn head(texts: &mut Texts, nl: &'static str, en: &'static str, button: &gtk::Button) -> gtk::Box {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    h.add_css_class("w11-start-head");
    let title = tlabel(texts, nl, en, "w11-start-title");
    title.set_hexpand(true);
    button.set_valign(gtk::Align::Center);
    h.append(&title);
    h.append(button);
    h
}

fn ellipsized(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l.set_xalign(0.0);
    l
}

/// Restart a CSS animation: switch between "<base>-a" and "<base>-b" (same keyframes, different names).
fn play(w: &impl IsA<gtk::Widget>, base: &str) {
    let (a, b) = (format!("{base}-a"), format!("{base}-b"));
    if w.has_css_class(&a) {
        w.remove_css_class(&a);
        w.add_css_class(&b);
    } else {
        w.remove_css_class(&b);
        w.add_css_class(&a);
    }
}

fn stop(w: &impl IsA<gtk::Widget>, base: &str) {
    w.remove_css_class(&format!("{base}-a"));
    w.remove_css_class(&format!("{base}-b"));
}

fn avatar(name: &str, image: Option<&str>, size: i32) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("w11-avatar");
    b.set_size_request(size, size);
    b.set_overflow(gtk::Overflow::Hidden);
    b.set_valign(gtk::Align::Center);
    if let Some(path) = image.filter(|p| std::path::Path::new(p).is_file()) {
        let p = gtk::Picture::for_filename(path);
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_can_shrink(true);
        p.set_size_request(size, size);
        b.append(&p);
        return b.upcast();
    }
    let initials: Vec<char> = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .filter(|c| c.is_alphanumeric())
        .collect();
    if let Some(first) = initials.first() {
        let mut s: String = first.to_uppercase().collect();
        if initials.len() > 1
            && let Some(last) = initials.last()
        {
            s.extend(last.to_uppercase());
        }
        let l = gtk::Label::new(Some(&s));
        l.add_css_class("w11-avatar-initials");
        l.set_vexpand(true);
        b.append(&l);
    } else {
        let g = crate::icons::glyph("user", size / 2);
        g.set_vexpand(true);
        b.append(&g);
    }
    b.upcast()
}

fn folder_glyph(f: &str) -> Option<&'static str> {
    Some(match f {
        "settings" => "settings",
        "explorer" => "folder",
        "documents" => "document",
        "downloads" => "download",
        "music" => "music",
        "pictures" => "image",
        "videos" => "video",
        "personal" => "user",
        "home" => "home",
        _ => return None,
    })
}

fn folder_label(shell: &Shell, f: &str) -> String {
    let (nl, en) = match f {
        "settings" => ("Instellingen", "Settings"),
        "explorer" => ("Verkenner", "File Explorer"),
        "documents" => ("Documenten", "Documents"),
        "downloads" => ("Downloads", "Downloads"),
        "music" => ("Muziek", "Music"),
        "pictures" => ("Afbeeldingen", "Pictures"),
        "videos" => ("Video's", "Videos"),
        "network" => ("Netwerk", "Network"),
        _ => ("Persoonlijke map", "Personal folder"),
    };
    shell.t(nl, en)
}

fn on_right_click(w: &impl IsA<gtk::Widget>, f: impl Fn(&gtk::Widget, f64, f64) + 'static) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak = w.as_ref().downgrade();
    click.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        if let Some(w) = weak.upgrade() {
            f(&w, x, y);
        }
    });
    w.add_controller(click);
}

/* ================================================================== Start */

impl Start {
    fn build(shell: &Rc<Shell>) -> Rc<Start> {
        let mut texts: Texts = Vec::new();
        let window = gtk::Window::builder()
            .decorated(false)
            .resizable(false)
            .default_width(642)
            .default_height(726)
            .title("Start")
            .hide_on_close(true)
            .build();
        window.set_application(Some(&shell.app));
        window.set_widget_name("start-window");
        window.add_css_class("w11-start-window");
        crate::wayland::set_app_id(&window, crate::START_ID);

        let root = gtk::Overlay::new();
        window.set_child(Some(&root));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("w11-start");
        panel.set_overflow(gtk::Overflow::Hidden);
        root.set_child(Some(&panel));
        let menu = MenuLayer::new(&root);

        /* search box */
        let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top.add_css_class("w11-start-top");
        let sbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        sbox.add_css_class("w11-sbox");
        let sicon = crate::icons::glyph("search", 16);
        sicon.add_css_class("w11-sbox-icon");
        sbox.append(&sicon);
        let text = gtk::Text::new();
        text.add_css_class("w11-sbox-input");
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.set_widget_name("start-search");
        let t2 = text.clone();
        texts.push(Box::new(move |s| {
            t2.set_placeholder_text(Some(&s.t(
                "Zoek naar apps, instellingen en documenten",
                "Search for apps, settings and documents",
            )))
        }));
        sbox.append(&text);
        let clear_btn = gtk::Button::new();
        clear_btn.add_css_class("w11-sbox-clear");
        clear_btn.set_child(Some(&crate::icons::glyph("close", 12)));
        clear_btn.set_focusable(false);
        clear_btn.set_visible(false);
        clear_btn.set_valign(gtk::Align::Center);
        clear_btn.set_widget_name("start-search-clear");
        let c2 = clear_btn.clone();
        texts.push(Box::new(move |s| c2.set_tooltip_text(Some(&s.t("Wissen", "Clear")))));
        sbox.append(&clear_btn);
        top.append(&sbox);
        let click = gtk::GestureClick::new();
        let t3 = text.downgrade();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(t) = t3.upgrade() {
                t.grab_focus();
            }
        });
        sbox.add_controller(click);

        let stack = gtk::Stack::new();
        stack.add_css_class("w11-start-main");
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::None);

        /* home */
        let home = gtk::Box::new(gtk::Orientation::Vertical, 0);
        home.add_css_class("w11-start-view");
        home.add_css_class("w11-start-home");
        let all_btn = pill(&mut texts, "Alle apps", "All apps", "chevron-right", true);
        all_btn.set_widget_name("start-all-apps");
        home.append(&head(&mut texts, "Vastgemaakt", "Pinned", &all_btn));
        let pin_grid = gtk::Grid::new();
        pin_grid.add_css_class("w11-pinned-page");
        pin_grid.set_halign(gtk::Align::Center);
        pin_grid.set_valign(gtk::Align::Start);
        let pinned_area = gtk::Overlay::new();
        pinned_area.add_css_class("w11-pinned");
        pinned_area.set_child(Some(&pin_grid));
        pinned_area.set_overflow(gtk::Overflow::Hidden);
        let dots = gtk::Box::new(gtk::Orientation::Vertical, 2);
        dots.add_css_class("w11-dots");
        dots.set_halign(gtk::Align::End);
        dots.set_valign(gtk::Align::Center);
        dots.set_margin_end(10);
        pinned_area.add_overlay(&dots);
        home.append(&pinned_area);
        let rec_section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        rec_section.add_css_class("w11-start-rec");
        let more_btn = pill(&mut texts, "Meer", "More", "chevron-right", true);
        more_btn.set_widget_name("start-more");
        rec_section.append(&head(&mut texts, "Aanbevolen", "Recommended", &more_btn));
        let rec_grid = gtk::Grid::new();
        rec_grid.add_css_class("w11-rec-grid");
        rec_grid.set_column_homogeneous(true);
        rec_grid.set_column_spacing(16);
        rec_grid.set_row_spacing(4);
        rec_section.append(&rec_grid);
        home.append(&rec_section);
        // On a short screen the extension makes Start lower than 726 px: the home view is cut, not the window grown.
        let home_clip = gtk::ScrolledWindow::new();
        home_clip.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
        home_clip.set_child(Some(&home));
        stack.add_named(&home_clip, Some("home"));

        /* all apps */
        let all = gtk::Box::new(gtk::Orientation::Vertical, 0);
        all.add_css_class("w11-start-view");
        all.add_css_class("w11-start-all");
        let all_back = pill(&mut texts, "Terug", "Back", "chevron-left", false);
        all_back.set_widget_name("start-all-back");
        all.append(&head(&mut texts, "Alle apps", "All apps", &all_back));
        let all_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        all_list.add_css_class("w11-allapps-list");
        let all_scroll = gtk::ScrolledWindow::new();
        all_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        all_scroll.set_child(Some(&all_list));
        all_scroll.add_css_class("w11-allapps");
        all_scroll.set_vexpand(true);
        let letters = gtk::Grid::new();
        letters.add_css_class("w11-letters");
        letters.set_halign(gtk::Align::Center);
        letters.set_valign(gtk::Align::Center);
        letters.set_column_spacing(4);
        letters.set_row_spacing(4);
        letters.set_visible(false);
        let wrap = gtk::Overlay::new();
        wrap.add_css_class("w11-allapps-wrap");
        wrap.set_vexpand(true);
        wrap.set_child(Some(&all_scroll));
        wrap.add_overlay(&letters);
        all.append(&wrap);
        stack.add_named(&all, Some("all"));

        /* recommended "Meer" */
        let recmore = gtk::Box::new(gtk::Orientation::Vertical, 0);
        recmore.add_css_class("w11-start-view");
        recmore.add_css_class("w11-start-recmore");
        let rm_back = pill(&mut texts, "Terug", "Back", "chevron-left", false);
        rm_back.set_widget_name("start-rec-back");
        recmore.append(&head(&mut texts, "Aanbevolen", "Recommended", &rm_back));
        let recmore_grid = gtk::Grid::new();
        recmore_grid.add_css_class("w11-rec-grid");
        recmore_grid.add_css_class("is-all");
        recmore_grid.set_column_homogeneous(true);
        recmore_grid.set_column_spacing(16);
        recmore_grid.set_row_spacing(4);
        recmore_grid.set_valign(gtk::Align::Start);
        let rm_scroll = gtk::ScrolledWindow::new();
        rm_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rm_scroll.set_child(Some(&recmore_grid));
        rm_scroll.add_css_class("w11-recmore-scroll");
        rm_scroll.set_vexpand(true);
        recmore.append(&rm_scroll);
        stack.add_named(&recmore, Some("rec"));

        /* search */
        let search = search::Search::new(shell);
        stack.add_named(&search.root, Some("search"));

        /* footer */
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("w11-start-footer");
        let user_btn = gtk::Button::new();
        user_btn.add_css_class("w11-start-user");
        user_btn.set_widget_name("start-user");
        user_btn.set_valign(gtk::Align::Center);
        footer.append(&user_btn);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        let foot_right = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        foot_right.add_css_class("w11-foot-right");
        foot_right.set_valign(gtk::Align::Center);
        footer.append(&foot_right);
        let power_btn = gtk::Button::new();
        power_btn.add_css_class("w11-footbtn");
        power_btn.set_widget_name("start-power");
        power_btn.set_child(Some(&crate::icons::glyph("power", 16)));
        let p2 = power_btn.clone();
        texts.push(Box::new(move |s| p2.set_tooltip_text(Some(&s.t("Aan/uit", "Power")))));

        // Only the middle grows: a vexpand child deep in the footer would otherwise split the spare height.
        top.set_vexpand(false);
        footer.set_vexpand(false);
        panel.append(&top);
        panel.append(&stack);
        panel.append(&footer);

        let start = Rc::new(Start {
            shell: shell.clone(),
            window,
            panel,
            top,
            menu,
            text,
            clear_btn,
            stack,
            view: Cell::new(View::Home),
            texts,
            lang: RefCell::new(shell.lang().to_owned()),
            pin_grid,
            dots,
            pinned_area,
            page: Cell::new(0),
            rec_grid,
            more_btn,
            all_list,
            all_scroll,
            letters,
            all_heads: RefCell::new(Vec::new()),
            all_dirty: Cell::new(true),
            recmore_grid,
            search,
            footer,
            user_btn,
            foot_right,
            power_btn,
            apps: RefCell::new(Vec::new()),
            browser: RefCell::new(None),
            rec_items: RefCell::new(Vec::new()),
            user: RefCell::new(User {
                name: glib::user_name().to_string_lossy().into_owned(),
                real_name: glib::real_name().to_string_lossy().into_owned(),
                avatar: None,
            }),
            apps_seq: Cell::new(0),
            apps_timer: RefCell::new(None),
            open: Cell::new(false),
            epoch: Cell::new(0),
            focused: Cell::new(false),
            focus_hide_at: Cell::new(None),
            glass_known: Cell::new(false),
            wheel_at: Cell::new(None),
        });
        // the buttons that need the Rc
        let w = Rc::downgrade(&start);
        all_btn.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.show_view(View::All, true);
            }
        });
        let w = Rc::downgrade(&start);
        start.more_btn.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.show_view(View::Rec, true);
            }
        });
        for b in [&all_back, &rm_back] {
            let w = Rc::downgrade(&start);
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.show_view(View::Home, true);
                }
            });
        }
        start
    }

    fn connect(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.clear_btn.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.text.set_text("");
                s.text.grab_focus();
            }
        });
        let w = Rc::downgrade(self);
        self.text.connect_changed(move |_| {
            if let Some(s) = w.upgrade() {
                s.on_query();
            }
        });
        let w = Rc::downgrade(self);
        self.text.connect_activate(move |_| {
            if let Some(s) = w.upgrade() {
                s.enter();
            }
        });
        let w = Rc::downgrade(self);
        self.user_btn.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.user_menu();
            }
        });
        let w = Rc::downgrade(self);
        self.power_btn.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.power_menu();
            }
        });

        // keyboard: Esc, typing anywhere searches, the search view's arrows and Enter
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| match w.upgrade() {
            Some(s) => s.key(key, state),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);

        // focus loss closes (after a short check, as the WebKit host)
        let w = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |win| {
            let Some(s) = w.upgrade() else { return };
            if win.is_active() {
                s.focused.set(true);
            } else if s.focused.get() && s.open.get() {
                s.dismiss_after_focus_loss();
            }
        });
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(s) = w.upgrade() {
                s.dismiss();
            }
            glib::Propagation::Stop
        });
        let w = Rc::downgrade(self);
        self.window.connect_map(move |win| {
            if let Some(s) = w.upgrade()
                && s.open.get()
                && let Some(surface) = win.surface()
            {
                surface.set_input_region(None);
            }
        });

        // wheel paging over the pinned grid
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |c, _, dy| {
            let Some(s) = w.upgrade() else { return glib::Propagation::Proceed };
            let threshold = if c.unit() == gdk::ScrollUnit::Surface { 4.0 } else { 0.01 };
            if dy.abs() < threshold {
                return glib::Propagation::Stop;
            }
            if s.wheel_at.get().is_some_and(|t| t.elapsed() < Duration::from_millis(350)) {
                return glib::Propagation::Stop;
            }
            s.wheel_at.set(Some(Instant::now()));
            let page = s.page.get();
            s.go_page(if dy > 0.0 { page + 1 } else { page.saturating_sub(1) }, false);
            glib::Propagation::Stop
        });
        self.pinned_area.add_controller(scroll);
    }

    /* -------------------------------------------------------- lifecycle */

    fn toggle(self: &Rc<Self>) {
        if self.open.get() {
            self.dismiss();
            return;
        }
        // The click on the Start button that took the focus away (and so closed Start) must not reopen it.
        if self.focus_hide_at.get().is_some_and(|t| t.elapsed() < Duration::from_millis(350)) {
            return;
        }
        self.show(None);
    }

    fn show(self: &Rc<Self>, query: Option<&str>) {
        if !self.open.get() {
            self.open.set(true);
            self.epoch.set(self.epoch.get() + 1);
            self.focused.set(false);
            self.menu.close();
            if let Some(surface) = self.window.surface() {
                surface.set_input_region(None);
            }
            if !self.glass_known.get() {
                self.configure_backdrop();
            }
            self.page.set(0);
            self.render_pinned(0);
            self.load_recommended();
            stop(&self.panel, "w11-drop");
            play(&self.panel, "w11-rise");
            for w in [self.top.upcast_ref::<gtk::Widget>(), self.stack.upcast_ref(), self.footer.upcast_ref()] {
                play(w, "w11-fadeup");
            }
            self.window.present();
            self.glass_phase("open");
            self.text.grab_focus();
        }
        if let Some(q) = query {
            if !q.is_empty() {
                self.text.set_text(q);
            }
            self.text.grab_focus();
            self.text.set_position(-1);
        }
    }

    fn dismiss(self: &Rc<Self>) {
        if !self.open.replace(false) {
            return;
        }
        let epoch = self.epoch.get() + 1;
        self.epoch.set(epoch);
        self.menu.close();
        self.search.cancel_pending();
        self.hide_letters();
        // Clicks and keys belong to what the user chose while the glass slides down.
        if let Some(surface) = self.window.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
        self.glass_phase("closing");
        stop(&self.panel, "w11-rise");
        play(&self.panel, "w11-drop");
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(170), move || {
            if let Some(s) = w.upgrade()
                && !s.open.get()
                && s.epoch.get() == epoch
            {
                s.window.set_visible(false);
                s.glass_phase("hidden");
                s.reset();
            }
        });
    }

    fn hide_now(self: &Rc<Self>) {
        self.open.set(false);
        self.epoch.set(self.epoch.get() + 1);
        self.menu.close();
        self.window.set_visible(false);
        self.glass_phase("hidden");
        self.reset();
    }

    /// After closing (startmenu.js onClose): empty box, home view, first page.
    fn reset(self: &Rc<Self>) {
        self.text.set_text("");
        self.clear_btn.set_visible(false);
        self.search.reset();
        self.page.set(0);
        if self.view.get() != View::Home {
            self.show_view(View::Home, false);
        }
    }

    fn dismiss_after_focus_loss(self: &Rc<Self>) {
        if !self.open.get() {
            return;
        }
        let epoch = self.epoch.get();
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            let Some(s) = w.upgrade() else { return };
            if !s.open.get() || s.epoch.get() != epoch || !s.window.is_visible() || s.window.is_active() {
                return;
            }
            // GTK's focus can lag the compositor during a quick remap: ask the extension first.
            let focused = s.shell.gnome.call("shell.info", &json!({})).ok().is_some_and(|i| i["startFocused"] == true);
            if !focused {
                s.focus_hide_at.set(Some(Instant::now()));
                s.dismiss();
            }
        });
    }

    /* ---------------------------------------------------------- backdrop */

    fn glass_phase(&self, phase: &str) {
        self.shell.gnome.send("nativeBackdrop.start", json!({"pid": std::process::id(), "phase": phase}));
    }

    fn configure_backdrop(&self) {
        let settings = self.shell.settings();
        let args = json!({
            "pid": std::process::id(), "kind": "start",
            "transparency": settings["theme"]["transparency"] != false,
            "highContrast": settings["accessibility"]["highContrast"] == true,
            "autoHide": settings["taskbar"]["autoHide"] == true,
        });
        match self.shell.gnome.call("nativeBackdrop.configure", &args) {
            Ok(state) => {
                self.glass_known.set(true);
                self.apply_backdrop(&state);
            }
            Err(_) => self.panel.remove_css_class("w11-glass"),
        }
    }

    fn apply_backdrop(&self, state: &Value) {
        let pid = std::process::id() as u64;
        let blur = state["available"] == true
            && state["windows"]
                .as_array()
                .is_some_and(|ws| ws.iter().any(|w| w["pid"].as_u64() == Some(pid) && w["kind"] == "start" && w["blur"] == true));
        let on = blur && self.shell.setting("theme.transparency") != false && self.shell.setting("accessibility.highContrast") != true;
        if on { self.panel.add_css_class("w11-glass") } else { self.panel.remove_css_class("w11-glass") }
    }

    /// "Accentkleur weergeven op Start en taakbalk" (dark mode only) and high contrast.
    fn apply_surface(&self) {
        let accent = self.shell.setting("theme.accentOnTaskbar") == true && self.shell.dark();
        if accent { self.panel.add_css_class("w11-accent-surface") } else { self.panel.remove_css_class("w11-accent-surface") }
        let hc = self.shell.setting("accessibility.highContrast") == true;
        if hc { self.panel.add_css_class("w11-hc") } else { self.panel.remove_css_class("w11-hc") }
    }

    fn on_settings(self: &Rc<Self>) {
        let lang = self.shell.lang().to_owned();
        let changed = *self.lang.borrow() != lang;
        if changed {
            *self.lang.borrow_mut() = lang;
            self.relabel();
            self.load_apps();
        }
        self.apply_surface();
        self.configure_backdrop();
        self.render_pinned(0);
        self.render_recommended();
        self.render_footer();
        if self.view.get() == View::All {
            self.render_all();
        } else {
            self.all_dirty.set(true);
        }
        self.search.refresh();
    }

    fn relabel(&self) {
        for f in &self.texts {
            f(&self.shell);
        }
    }

    /* -------------------------------------------------------------- data */

    fn app(&self, id: &str) -> Option<App> {
        self.apps.borrow().iter().find(|a| a.id == id).cloned()
    }

    fn pins(&self) -> Vec<String> {
        data::start_pins(&self.shell, &self.apps.borrow(), self.browser.borrow().as_deref())
    }

    fn pinned_apps(&self) -> Vec<App> {
        let apps = self.apps.borrow();
        self.pins().iter().filter_map(|id| apps.iter().find(|a| &a.id == id).cloned()).collect()
    }

    fn load_apps_soon(self: &Rc<Self>) {
        if let Some(id) = self.apps_timer.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(s) = w.upgrade() {
                // The source has run: forget its id without removing it.
                if let Some(id) = s.apps_timer.borrow_mut().take() {
                    std::mem::forget(id);
                }
                s.load_apps();
            }
        });
        *self.apps_timer.borrow_mut() = Some(id);
    }

    fn load_apps(self: &Rc<Self>) {
        self.shell.backend.set_lang(self.shell.lang());
        let seq = self.apps_seq.get() + 1;
        self.apps_seq.set(seq);
        let w = Rc::downgrade(self);
        self.shell.call("apps.list", json!({}), move |result| {
            let Some(s) = w.upgrade() else { return };
            if s.apps_seq.get() != seq {
                return;
            }
            match result {
                Ok(v) => {
                    let list: Vec<App> = v.as_array().map(|a| a.iter().filter_map(App::from_json).collect()).unwrap_or_default();
                    s.search.set_apps(&list);
                    *s.apps.borrow_mut() = list;
                }
                Err(e) => eprintln!("win11-shell: apps.list: {e}"),
            }
            s.all_dirty.set(true);
            s.render_pinned(0);
            if s.view.get() == View::All {
                s.render_all();
            }
            if s.open.get() {
                s.load_recommended();
            }
            s.search.refresh();
        });
        let w = Rc::downgrade(self);
        self.shell.call("apps.defaults", json!({}), move |result| {
            let Some(s) = w.upgrade() else { return };
            let Ok(d) = result else { return };
            *s.browser.borrow_mut() = d["browser"]["id"].as_str().filter(|x| !x.is_empty()).map(str::to_owned);
            let ids: HashSet<String> = d
                .as_object()
                .map(|o| o.values().filter_map(|x| x["id"].as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            *s.search.default_ids.borrow_mut() = ids;
            s.render_pinned(0);
        });
    }

    fn load_user(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.shell.call("system.info", json!({}), move |result| {
            let Some(s) = w.upgrade() else { return };
            let Ok(info) = result else { return };
            let u = &info["user"];
            {
                let mut user = s.user.borrow_mut();
                if let Some(n) = u["name"].as_str().filter(|x| !x.is_empty()) {
                    user.name = n.to_owned();
                }
                if let Some(n) = u["realName"].as_str().filter(|x| !x.is_empty()) {
                    user.real_name = n.to_owned();
                }
                user.avatar = u["avatar"].as_str().filter(|x| !x.is_empty()).map(str::to_owned);
            }
            s.render_footer();
        });
    }

    fn load_recommended(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.shell.call("apps.recent", json!({"limit": 24}), move |result| {
            let Some(s) = w.upgrade() else { return };
            let files: Vec<FileItem> =
                result.ok().and_then(|v| v.as_array().map(|a| a.iter().filter_map(FileItem::from_json).collect())).unwrap_or_default();
            let since = data::now_ms() - 14 * 86_400_000;
            let mut fresh: Vec<App> = s.apps.borrow().iter().filter(|a| a.installed_at > since).cloned().collect();
            fresh.sort_by(|a, b| b.installed_at.cmp(&a.installed_at));
            let mut items: Vec<RecItem> = fresh
                .into_iter()
                .take(4)
                .map(|a| RecItem { key: format!("app:{}", a.id), title: a.name.clone(), time: a.installed_at, kind: RecKind::App(a) })
                .collect();
            items.extend(
                files
                    .into_iter()
                    .map(|f| RecItem { key: format!("file:{}", f.path), title: f.name.clone(), time: f.modified, kind: RecKind::File(f) }),
            );
            items.sort_by(|a, b| b.time.cmp(&a.time));
            *s.rec_items.borrow_mut() = items;
            s.render_recommended();
        });
    }

    /// start.layout: rows of pins and of recommendations; start.showRecommended.
    fn layout(&self) -> (usize, usize, bool) {
        let (pin, rec) = match self.shell.setting("start.layout").as_str() {
            Some("morePins" | "more-pins") => (4, 2),
            Some("moreRecommendations" | "more-recommendations") => (2, 4),
            _ => (3, 3),
        };
        (pin, rec, self.shell.setting("start.showRecommended") != false)
    }

    /* ------------------------------------------------------------ views */

    fn show_view(self: &Rc<Self>, next: View, animate: bool) {
        let prev = self.view.get();
        if next == prev {
            return;
        }
        self.view.set(next);
        self.hide_letters();
        self.menu.close();
        if next == View::All {
            if self.all_dirty.get() {
                self.render_all();
            }
            self.all_scroll.vadjustment().set_value(0.0);
        }
        self.footer.set_visible(next != View::Search);
        if next == View::Search { self.panel.add_css_class("is-searching") } else { self.panel.remove_css_class("is-searching") }
        self.stack.set_visible_child_name(next.name());
        if let Some(child) = self.stack.visible_child() {
            for base in ["w11-from-right", "w11-from-left", "w11-fade"] {
                stop(&child, base);
            }
            if animate {
                if next != View::Search && prev != View::Search {
                    play(&child, if next == View::Home { "w11-from-left" } else { "w11-from-right" });
                } else {
                    play(&child, "w11-fade");
                }
            }
        }
        if animate && next != View::Search && !self.text.has_focus() {
            let first = match next {
                View::All => self.all_list.first_child().and_then(|h| h.next_sibling()),
                View::Rec => self.recmore_grid.first_child(),
                _ => None,
            };
            if let Some(f) = first {
                f.grab_focus();
            }
        }
    }

    fn on_query(self: &Rc<Self>) {
        let q = self.text.text().to_string();
        self.clear_btn.set_visible(!q.is_empty());
        if !q.trim().is_empty() {
            if self.view.get() != View::Search {
                self.show_view(View::Search, true);
            }
            self.search.set_query(&q);
        } else if self.view.get() == View::Search {
            self.search.set_query("");
            self.show_view(View::Home, true);
        }
    }

    fn enter(self: &Rc<Self>) {
        if self.text.text().trim().is_empty() {
            return;
        }
        if self.view.get() != View::Search {
            self.on_query();
        }
        self.search.activate();
    }

    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
        let in_text = focus.as_ref().is_some_and(|f| f == self.text.upcast_ref::<gtk::Widget>());
        if key == gdk::Key::Escape {
            if self.menu.close() {
                return Stop;
            }
            if self.hide_letters() {
                if let Some((_, h)) = self.all_heads.borrow().first() {
                    h.grab_focus();
                }
                return Stop;
            }
            self.dismiss();
            return Stop;
        }
        if self.menu.is_open() {
            return Proceed;
        }
        let in_search = focus.as_ref().is_some_and(|f| f.is_ancestor(&self.search.root));
        if self.view.get() == View::Search
            && (in_text || in_search)
            && !(in_text && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter))
            && self.search.handle_key(key, state, in_text)
        {
            return Stop;
        }
        if in_text {
            if matches!(key, gdk::Key::Down | gdk::Key::KP_Down) && self.text.text().is_empty() {
                let first = match self.view.get() {
                    View::Home => self.pin_grid.first_child(),
                    View::All => self.all_list.first_child().and_then(|h| h.next_sibling()),
                    View::Rec => self.recmore_grid.first_child(),
                    View::Search => None,
                };
                if let Some(f) = first {
                    f.grab_focus();
                    return Stop;
                }
            }
            return Proceed;
        }
        let name = focus.as_ref().map(|f| f.widget_name().to_string()).unwrap_or_default();
        let menu_key = key == gdk::Key::Menu || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK));
        if let Some(id) = name.strip_prefix("start-pin-") {
            match key {
                gdk::Key::Page_Down => {
                    self.go_page(self.page.get() + 1, true);
                    return Stop;
                }
                gdk::Key::Page_Up => {
                    self.go_page(self.page.get().saturating_sub(1), true);
                    return Stop;
                }
                _ if menu_key => {
                    if let (Some(app), Some(f)) = (self.app(id), focus.clone()) {
                        self.tile_menu(&app, Anchor::Over(f));
                    }
                    return Stop;
                }
                _ => {}
            }
        }
        if menu_key && let Some(id) = name.strip_prefix("start-app-") {
            if let (Some(app), Some(f)) = (self.app(id), focus.clone()) {
                self.all_menu(&app, Anchor::Over(f));
            }
            return Stop;
        }
        if menu_key && let Some(i) = name.strip_prefix("start-rec-").and_then(|i| i.parse::<usize>().ok()) {
            let item = self.rec_items.borrow().get(i).cloned();
            if let (Some(item), Some(f)) = (item, focus.clone()) {
                self.rec_menu(&item, Anchor::Over(f));
            }
            return Stop;
        }
        if key == gdk::Key::BackSpace && self.view.get() == View::All && (name.starts_with("start-app-") || name.starts_with("start-letter-")) {
            self.show_view(View::Home, true);
            return Stop;
        }
        // typing anywhere starts a search (Win11)
        let plain = !state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK);
        if plain
            && let Some(c) = key.to_unicode()
            && !c.is_control()
            && c != ' '
        {
            self.text.grab_focus();
            let mut s = self.text.text().to_string();
            s.push(c);
            self.text.set_text(&s);
            self.text.set_position(-1);
            return Stop;
        }
        if key == gdk::Key::BackSpace && self.view.get() == View::Search {
            self.text.grab_focus();
        }
        Proceed
    }

    /* ------------------------------------------------------------ pinned */

    fn render_pinned(self: &Rc<Self>, dir: i32) {
        let (rows, _, _) = self.layout();
        let per = COLS * rows;
        let apps = self.pinned_apps();
        let pages = apps.len().div_ceil(per).max(1);
        let page = self.page.get().min(pages - 1);
        self.page.set(page);
        clear(&self.pin_grid);
        for (i, app) in apps.iter().skip(page * per).take(per).enumerate() {
            let tile = self.tile(app, page * per + i);
            self.pin_grid.attach(&tile, (i % COLS) as i32, (i / COLS) as i32, 1, 1);
        }
        if apps.is_empty() && !self.apps.borrow().is_empty() {
            let l = gtk::Label::new(Some(&self.shell.t(
                "Maak apps vast door in Alle apps met de rechtermuisknop te klikken.",
                "Pin apps by right-clicking them in All apps.",
            )));
            l.add_css_class("w11-pinned-empty");
            l.set_wrap(true);
            l.set_justify(gtk::Justification::Center);
            l.set_size_request(COLS as i32 * TILE_W, -1);
            self.pin_grid.attach(&l, 0, 0, COLS as i32, 1);
        }
        self.pinned_area.set_size_request(-1, rows as i32 * TILE_H);
        clear(&self.dots);
        self.dots.set_visible(pages > 1);
        for i in 0..pages {
            let d = gtk::Button::new();
            d.add_css_class("w11-dot");
            d.set_focusable(false);
            d.set_size_request(12, 12);
            if i == page {
                d.add_css_class("is-active");
            }
            let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            mark.add_css_class("w11-dot-mark");
            mark.set_halign(gtk::Align::Center);
            mark.set_valign(gtk::Align::Center);
            d.set_child(Some(&mark));
            d.set_tooltip_text(Some(&self.shell.t(&format!("Pagina {}", i + 1), &format!("Page {}", i + 1))));
            let w = Rc::downgrade(self);
            d.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.go_page(i, false);
                }
            });
            self.dots.append(&d);
        }
        if dir != 0 {
            play(&self.pin_grid, if dir > 0 { "w11-page-next" } else { "w11-page-prev" });
        }
    }

    fn go_page(self: &Rc<Self>, i: usize, focus_first: bool) -> bool {
        let (rows, _, _) = self.layout();
        let pages = self.pinned_apps().len().div_ceil(COLS * rows).max(1);
        let next = i.min(pages - 1);
        let page = self.page.get();
        if next == page {
            return false;
        }
        self.page.set(next);
        self.render_pinned(if next > page { 1 } else { -1 });
        if focus_first && let Some(f) = self.pin_grid.first_child() {
            f.grab_focus();
        }
        true
    }

    fn tile(self: &Rc<Self>, app: &App, index: usize) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("w11-tile");
        b.set_widget_name(&format!("start-pin-{}", app.id));
        b.set_size_request(TILE_W, TILE_H);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let icon = crate::icons::app_icon(&app.icon, 32);
        icon.add_css_class("w11-tile-icon");
        col.append(&icon);
        let l = ellipsized(&app.name, "w11-tile-label");
        l.set_xalign(0.5);
        col.append(&l);
        b.set_child(Some(&col));
        b.set_tooltip_text(Some(&app.name));
        let id = app.id.clone();
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.launch(&id);
            }
        });
        let a = app.clone();
        let w = Rc::downgrade(self);
        on_right_click(&b, move |widget, x, y| {
            if let Some(s) = w.upgrade() {
                s.tile_menu(&a, Anchor::At(widget.clone(), x, y));
            }
        });
        // drag a tile onto another position
        let src = gtk::DragSource::new();
        src.set_actions(gdk::DragAction::MOVE);
        let id = app.id.clone();
        src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
        src.connect_drag_begin(|src, _| {
            if let Some(w) = src.widget() {
                w.add_css_class("is-dragging");
                let paintable = gtk::WidgetPaintable::new(Some(&w));
                src.set_icon(Some(&paintable), w.width() / 2, w.height() / 2);
            }
        });
        src.connect_drag_end(|src, _, _| {
            if let Some(w) = src.widget() {
                w.remove_css_class("is-dragging");
            }
        });
        b.add_controller(src);
        let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(s), Ok(id)) = (w.upgrade(), value.get::<String>()) else { return false };
            s.move_pin(&id, index);
            true
        });
        b.add_controller(drop);
        b
    }

    fn move_pin(&self, id: &str, index: usize) {
        let mut l: Vec<String> = self.pins().into_iter().filter(|x| x != id).collect();
        l.insert(index.min(l.len()), id.to_owned());
        data::set_start_pins(&self.shell, l);
    }

    fn launch(self: &Rc<Self>, id: &str) {
        self.dismiss();
        data::launch(&self.shell, id);
    }

    /* ------------------------------------------------------------- menus */

    fn pin_items(self: &Rc<Self>, app: &App, front: bool) -> Vec<Item> {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let pins = self.pins();
        let mut items = Vec::new();
        let id = app.id.clone();
        if front && pins.iter().position(|x| *x == id).is_some_and(|i| i > 0) {
            let w = Rc::downgrade(self);
            let i = id.clone();
            items.push(Item::action(t("Naar voren verplaatsen", "Move to front"), "arrow-up", "start-menu-front", move || {
                if let Some(s) = w.upgrade() {
                    s.page.set(0);
                    let mut l: Vec<String> = s.pins().into_iter().filter(|x| *x != i).collect();
                    l.insert(0, i.clone());
                    data::set_start_pins(&s.shell, l);
                }
            }));
        }
        let i = id.clone();
        items.push(if pins.contains(&id) {
            Item::action(t("Losmaken van Start", "Unpin from Start"), "unpin", "start-menu-unpin-start", move || unpin_start(&i))
        } else {
            Item::action(t("Vastmaken aan Start", "Pin to Start"), "pin", "start-menu-pin-start", move || pin_start(&i))
        });
        let shell = self.shell.clone();
        items.push(if data::taskbar_pins(&shell).contains(&id) {
            Item::action(t("Losmaken van taakbalk", "Unpin from taskbar"), "unpin", "start-menu-unpin-taskbar", move || {
                data::set_taskbar_pins(&shell, data::taskbar_pins(&shell).into_iter().filter(|x| *x != id).collect())
            })
        } else {
            Item::action(t("Vastmaken aan taakbalk", "Pin to taskbar"), "pin", "start-menu-pin-taskbar", move || {
                let mut l: Vec<String> = data::taskbar_pins(&shell).into_iter().filter(|x| *x != id).collect();
                l.push(id.clone());
                data::set_taskbar_pins(&shell, l);
            })
        });
        items
    }

    /// An app menu, then "Verwijderen" once software.uninstallInfo answered (core/software.js).
    fn app_menu(self: &Rc<Self>, items: Vec<Item>, app: &App, anchor: Anchor) {
        let menu = self.menu.show(items, anchor);
        let w = Rc::downgrade(self);
        let id = app.id.clone();
        self.shell.call("software.uninstallInfo", json!({"id": app.id}), move |result| {
            let Some(s) = w.upgrade() else { return };
            let t = |nl: &str, en: &str| s.shell.t(nl, en);
            let item = match result {
                Ok(info) => {
                    let label = if info["action"] == "removeShortcut" {
                        t("Snelkoppeling verwijderen", "Remove shortcut")
                    } else {
                        t("Verwijderen", "Uninstall")
                    };
                    if info["supported"] == true {
                        let w = Rc::downgrade(&s);
                        let id = id.clone();
                        Item::action(label, "delete", "start-menu-uninstall", move || {
                            if let Some(s) = w.upgrade() {
                                s.dismiss();
                                data::open_app(&s.shell, "installer", json!({"uninstall": id}));
                            }
                        })
                    } else {
                        let reason = info["reason"].as_str().filter(|r| !r.is_empty()).map(str::to_owned);
                        Item::disabled(label, "delete", "start-menu-uninstall", reason.or_else(|| Some(t("Niet beschikbaar", "Not available"))))
                    }
                }
                Err(e) => Item::disabled(t("Verwijderen", "Uninstall"), "delete", "start-menu-uninstall", Some(e)),
            };
            s.menu.append(&menu, Item::Separator);
            s.menu.append(&menu, item);
        });
    }

    fn tile_menu(self: &Rc<Self>, app: &App, anchor: Anchor) {
        let items = self.pin_items(app, true);
        self.app_menu(items, app, anchor);
    }

    fn all_menu(self: &Rc<Self>, app: &App, anchor: Anchor) {
        let items = self.pin_items(app, false);
        self.app_menu(items, app, anchor);
    }

    fn rec_menu(self: &Rc<Self>, item: &RecItem, anchor: Anchor) {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let w = Rc::downgrade(self);
        let it = item.clone();
        let open = Item::action(t("Openen", "Open"), "open", "start-menu-open", move || {
            if let Some(s) = w.upgrade() {
                s.open_rec(&it);
            }
        });
        match &item.kind {
            RecKind::App(app) => {
                let i = app.id.clone();
                let pin = if self.pins().contains(&app.id) {
                    Item::action(t("Losmaken van Start", "Unpin from Start"), "unpin", "start-menu-unpin-start", move || unpin_start(&i))
                } else {
                    Item::action(t("Vastmaken aan Start", "Pin to Start"), "pin", "start-menu-pin-start", move || pin_start(&i))
                };
                self.app_menu(vec![open, pin], app, anchor);
            }
            RecKind::File(f) => {
                let w = Rc::downgrade(self);
                let (dir, path) = (data::parent_dir(&f.path), f.path.clone());
                let location = Item::action(t("Bestandslocatie openen", "Open file location"), "folder-open", "start-menu-location", move || {
                    if let Some(s) = w.upgrade() {
                        s.dismiss();
                        data::open_app(&s.shell, "explorer", json!({"path": dir, "select": path}));
                    }
                });
                let path = f.path.clone();
                let win = self.window.downgrade();
                let copy = Item::action(t("Pad kopiëren", "Copy path"), "copy", "start-menu-copy", move || {
                    if let Some(win) = win.upgrade() {
                        win.clipboard().set_text(&path);
                    }
                });
                self.menu.show(vec![open, location, copy], anchor);
            }
        }
    }

    fn user_menu(self: &Rc<Self>) {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let name = {
            let u = self.user.borrow().display();
            if u.is_empty() { t("Gebruiker", "User") } else { u }
        };
        let w = Rc::downgrade(self);
        let account = Item::action(t("Accountinstellingen wijzigen", "Change account settings"), "user", "start-menu-account", move || {
            if let Some(s) = w.upgrade() {
                s.dismiss();
                data::open_app(&s.shell, "settings", json!({"page": "accounts"}));
            }
        });
        let w = Rc::downgrade(self);
        let lock = Item::action(t("Vergrendelen", "Lock"), "lock", "start-menu-lock", move || {
            if let Some(s) = w.upgrade() {
                s.power("lock");
            }
        });
        let w = Rc::downgrade(self);
        let logout = Item::action(t("Afmelden", "Sign out"), "signout", "start-menu-logout", move || {
            if let Some(s) = w.upgrade() {
                s.power("logout");
            }
        });
        self.menu.show(vec![Item::Header(name), account, Item::Separator, lock, logout], Anchor::Above(self.user_btn.clone().upcast(), false));
    }

    fn power_menu(self: &Rc<Self>) {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let entry = |label: String, glyph: &'static str, action: &'static str| {
            let w = Rc::downgrade(self);
            Item::action(label, glyph, &format!("start-menu-{action}"), move || {
                if let Some(s) = w.upgrade() {
                    s.power(action);
                }
            })
        };
        let items = vec![
            entry(t("Slaapstand", "Sleep"), "sleep", "suspend"),
            entry(t("Afsluiten", "Shut down"), "power", "shutdown"),
            entry(t("Opnieuw opstarten", "Restart"), "restart", "reboot"),
        ];
        self.menu.show(items, Anchor::Above(self.power_btn.clone().upcast(), true));
    }

    /// A power action: Start closes first, then the panel confirms it full-screen (D-G58).
    fn power(self: &Rc<Self>, action: &'static str) {
        self.dismiss();
        let shell = self.shell.clone();
        glib::idle_add_local_once(move || crate::flyouts::power::confirm(&shell, action));
    }

    /* ------------------------------------------------------- recommended */

    fn render_recommended(self: &Rc<Self>) {
        let (_, rec, show) = self.layout();
        clear(&self.rec_grid);
        clear(&self.recmore_grid);
        let items = self.rec_items.borrow().clone();
        self.more_btn.set_visible(show && items.len() > rec * 2);
        if !show {
            let empty = self.rec_empty(&self.shell.t(
                "Zet aanbevelingen aan om hier je recente bestanden en nieuwe apps te zien.",
                "Turn on recommendations to see your recent files and new apps here.",
            ));
            let link = gtk::Button::with_label(&self.shell.t("Instellingen voor Start", "Start settings"));
            link.add_css_class("w11-linkbtn");
            link.set_halign(gtk::Align::Center);
            link.set_widget_name("start-rec-settings");
            let w = Rc::downgrade(self);
            link.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.dismiss();
                    data::open_app(&s.shell, "settings", json!({"page": "start"}));
                }
            });
            empty.append(&link);
            self.rec_grid.attach(&empty, 0, 0, 2, 1);
            return;
        }
        if items.is_empty() {
            let empty = self.rec_empty(&self.shell.t(
                "Hoe meer je je computer gebruikt, hoe meer recente bestanden en nieuwe apps je hier ziet.",
                "The more you use your computer, the more recent files and new apps you’ll see here.",
            ));
            self.rec_grid.attach(&empty, 0, 0, 2, 1);
            return;
        }
        for (i, item) in items.iter().take(rec * 2).enumerate() {
            self.rec_grid.attach(&self.rec_el(item, i), (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        for (i, item) in items.iter().enumerate() {
            self.recmore_grid.attach(&self.rec_el(item, i), (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
    }

    fn rec_empty(&self, text: &str) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.add_css_class("w11-rec-empty");
        b.set_size_request(-1, 116);
        b.set_valign(gtk::Align::Center);
        let l = gtk::Label::new(Some(text));
        l.set_wrap(true);
        l.set_justify(gtk::Justification::Center);
        l.set_vexpand(true);
        l.set_valign(gtk::Align::End);
        b.append(&l);
        b
    }

    fn rec_el(self: &Rc<Self>, item: &RecItem, i: usize) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("w11-rec");
        b.set_widget_name(&format!("start-rec-{i}"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let (icon, sub): (gtk::Widget, String) = match &item.kind {
            RecKind::App(a) => (crate::icons::app_icon(&a.icon, 32).upcast(), self.shell.t("Onlangs toegevoegd", "Recently added")),
            RecKind::File(f) => (search::file_icon(f, 32), data::when_label(&self.shell, f.modified)),
        };
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&ellipsized(&item.title, "w11-rec-title"));
        text.append(&ellipsized(&sub, "w11-rec-sub"));
        row.append(&text);
        b.set_child(Some(&row));
        let it = item.clone();
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.open_rec(&it);
            }
        });
        let it = item.clone();
        let w = Rc::downgrade(self);
        on_right_click(&b, move |widget, x, y| {
            if let Some(s) = w.upgrade() {
                s.rec_menu(&it, Anchor::At(widget.clone(), x, y));
            }
        });
        b
    }

    fn open_rec(self: &Rc<Self>, item: &RecItem) {
        match &item.kind {
            RecKind::App(a) => self.launch(&a.id),
            RecKind::File(f) => {
                self.dismiss();
                self.shell.fire("files.open", json!({"path": f.path}));
            }
        }
    }

    /* ---------------------------------------------------------- all apps */

    fn render_all(self: &Rc<Self>) {
        self.all_dirty.set(false);
        let mut apps = self.apps.borrow().clone();
        apps.sort_by_cached_key(|a| data::sort_key(&a.name));
        clear(&self.all_list);
        let mut heads = Vec::new();
        let mut has: HashSet<char> = HashSet::new();
        for letter in LETTERS.chars() {
            let group: Vec<&App> = apps.iter().filter(|a| data::letter_of(&a.name) == letter).collect();
            if group.is_empty() {
                continue;
            }
            has.insert(letter);
            let h = gtk::Button::with_label(&letter.to_string());
            h.add_css_class("w11-letter");
            h.set_halign(gtk::Align::Start);
            h.set_widget_name(&format!("start-letter-{letter}"));
            let w = Rc::downgrade(self);
            h.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.show_letters();
                }
            });
            self.all_list.append(&h);
            heads.push((letter, h));
            for app in group {
                let row = gtk::Button::new();
                row.add_css_class("w11-allapp");
                row.set_widget_name(&format!("start-app-{}", app.id));
                let r = gtk::Box::new(gtk::Orientation::Horizontal, 16);
                r.append(&crate::icons::app_icon(&app.icon, 24));
                r.append(&ellipsized(&app.name, "w11-allapp-name"));
                row.set_child(Some(&r));
                let id = app.id.clone();
                let w = Rc::downgrade(self);
                row.connect_clicked(move |_| {
                    if let Some(s) = w.upgrade() {
                        s.launch(&id);
                    }
                });
                let a = app.clone();
                let w = Rc::downgrade(self);
                on_right_click(&row, move |widget, x, y| {
                    if let Some(s) = w.upgrade() {
                        s.all_menu(&a, Anchor::At(widget.clone(), x, y));
                    }
                });
                self.all_list.append(&row);
            }
        }
        *self.all_heads.borrow_mut() = heads;
        clear(&self.letters);
        for (i, letter) in LETTERS.chars().enumerate() {
            let b = gtk::Button::with_label(&letter.to_string());
            b.add_css_class("w11-letters-btn");
            b.set_size_request(64, 52);
            b.set_sensitive(has.contains(&letter));
            b.set_widget_name(&format!("start-jump-{letter}"));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.jump_to(letter);
                }
            });
            self.letters.attach(&b, (i % 4) as i32, (i / 4) as i32, 1, 1);
        }
    }

    fn show_letters(self: &Rc<Self>) {
        self.letters.set_visible(true);
        self.all_scroll.add_css_class("is-dimmed");
        self.all_scroll.set_can_target(false);
        play(&self.letters, "w11-zoom");
        let mut child = self.letters.first_child();
        while let Some(c) = child {
            if c.is_sensitive() {
                c.grab_focus();
                break;
            }
            child = c.next_sibling();
        }
    }

    fn hide_letters(&self) -> bool {
        if !self.letters.is_visible() {
            return false;
        }
        self.letters.set_visible(false);
        self.all_scroll.remove_css_class("is-dimmed");
        self.all_scroll.set_can_target(true);
        true
    }

    fn jump_to(self: &Rc<Self>, letter: char) {
        self.hide_letters();
        let head = self.all_heads.borrow().iter().find(|(l, _)| *l == letter).map(|(_, h)| h.clone());
        if let Some(h) = head {
            if let Some(p) = h.compute_point(&self.all_list, &gtk::graphene::Point::new(0.0, 0.0)) {
                self.all_scroll.vadjustment().set_value((p.y() as f64 - 4.0).max(0.0));
            }
            h.grab_focus();
        }
    }

    /* ------------------------------------------------------------ footer */

    fn render_footer(self: &Rc<Self>) {
        let user = self.user.borrow().clone();
        let name = {
            let n = user.display();
            if n.is_empty() { self.shell.t("Gebruiker", "User") } else { n }
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&avatar(&name, user.avatar.as_deref(), 32));
        row.append(&ellipsized(&name, "w11-start-user-name"));
        self.user_btn.set_child(Some(&row));
        self.user_btn.set_tooltip_text(Some(&name));
        clear(&self.foot_right);
        let folders = match self.shell.setting("start.folders") {
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
            _ => vec!["settings".to_owned(), "explorer".into(), "documents".into(), "downloads".into()],
        };
        for f in folders {
            let Some(glyph) = folder_glyph(&f) else { continue };
            let b = gtk::Button::new();
            b.add_css_class("w11-footbtn");
            b.set_widget_name(&format!("start-folder-{f}"));
            b.set_child(Some(&crate::icons::glyph(glyph, 16)));
            b.set_tooltip_text(Some(&folder_label(&self.shell, &f)));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.open_folder(&f);
                }
            });
            self.foot_right.append(&b);
        }
        self.foot_right.append(&self.power_btn);
    }

    fn open_folder(self: &Rc<Self>, f: &str) {
        self.dismiss();
        match f {
            "settings" => data::open_app(&self.shell, "settings", json!({})),
            "explorer" => data::open_app(&self.shell, "explorer", json!({})),
            other => data::open_app(&self.shell, "explorer", json!({"place": if other == "personal" { "home" } else { other }})),
        }
    }
}
