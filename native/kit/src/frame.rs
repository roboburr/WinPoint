//! The Windows 11 app window (6ebf169f:vendor/ui/js/apps/appframe.js, css/appframe.css; the calendar's title bar is
//! the model): an undecorated gtk::ApplicationWindow with its own title bar (32 px by default) holding an
//! optional Back button, the app icon and title (or the app's own title widget), a slot for the app's title bar
//! content (Explorer's tabs, a search box) and the three caption buttons with the Win11 glyphs (close turns
//! red); dragged by the title bar (gtk::WindowHandle: double-click maximises, right-click is the window menu),
//! resized from invisible edges, square corners while maximized, tiled or fullscreen, the Mica background with
//! the extension's blur (`w11-glass`), the size and maximized state remembered per key in
//! $XDG_CONFIG_HOME/win11-gnome/native-windows.json, and an overlay layer for dialogs and toasts.
//!
//! Widget names for the dev channel: "<kind>-window", "<kind>-minimize", "<kind>-maximize", "<kind>-close",
//! "<kind>-back".
use crate::i18n::{self, t};
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib, graphene};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// How an AppWindow starts.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    /// The key its size is remembered by ("main", "properties", ...); empty: not remembered.
    pub key: String,
    /// The title (also the GTK title); empty: the app's name from its spec.
    pub title: String,
    /// The title bar icon: a glyph name or an app icon name; empty: the spec's icon; "-": none.
    pub icon: String,
    /// The default size when nothing is remembered.
    pub width: i32,
    pub height: i32,
    /// The smallest size.
    pub min_width: i32,
    pub min_height: i32,
    /// The title bar height (32; Explorer about 40 with tabs; 48 for a tall bar with Back as Settings has).
    pub titlebar_height: i32,
    /// Show the Back button in the title bar (appframe.js `back: true`).
    pub back: bool,
}

impl Default for WindowOptions {
    fn default() -> WindowOptions {
        WindowOptions {
            key: "main".into(),
            title: String::new(),
            icon: String::new(),
            width: 1100,
            height: 720,
            min_width: 500,
            min_height: 360,
            titlebar_height: 32,
            back: false,
        }
    }
}

struct Inner {
    window: gtk::ApplicationWindow,
    kind: &'static str,
    key: String,
    /// The root overlay: the layer overlay and the resize edges.
    root: gtk::Overlay,
    /// Dialogs and toasts go here, over the whole frame.
    layer: gtk::Overlay,
    frame: gtk::Box,
    titlebar: gtk::WindowHandle,
    titlebar_box: gtk::Box,
    back: gtk::Button,
    icon_slot: gtk::Box,
    title_label: gtk::Label,
    left: gtk::Box,
    title_widget: RefCell<Option<gtk::Widget>>,
    center: gtk::Box,
    end: gtk::Box,
    max_button: gtk::Button,
    max_glyph: gtk::Image,
    content: gtk::Box,
    toasts: gtk::Box,
    edges: Vec<gtk::Widget>,
    dialogs: RefCell<Vec<gtk::Widget>>,
    shortcuts: RefCell<Vec<Shortcut>>,
    on_back: RefCell<Option<Rc<dyn Fn()>>>,
    title_text: RefCell<String>,
}

struct Shortcut {
    accel: String,
    key: gdk::Key,
    mods: gdk::ModifierType,
    f: Rc<dyn Fn() -> bool>,
}

/// The Windows 11 app window. Cheap to clone (a handle).
#[derive(Clone)]
pub struct AppWindow {
    inner: Rc<Inner>,
}

thread_local! {
    static WINDOWS: RefCell<Vec<Weak<Inner>>> = const { RefCell::new(Vec::new()) };
}

impl AppWindow {
    /// A new window of the running app (kit::ctx().app()); call `present()` once its content is set.
    pub fn new(opts: WindowOptions) -> AppWindow {
        let ctx = crate::ctx::ctx();
        let spec = ctx.spec().clone();
        let window = gtk::ApplicationWindow::builder().application(ctx.app()).decorated(false).build();
        window.add_css_class("w11-app-window");
        window.add_css_class(&format!("{}-window", spec.kind));
        window.set_widget_name(&format!("{}-window", spec.kind));
        crate::theme::style_window(window.upcast_ref());
        let (w, h, maximized) = load_size(&key_of(spec.kind, &opts.key)).unwrap_or((opts.width, opts.height, false));
        window.set_default_size(w.max(opts.min_width), h.max(opts.min_height));
        if maximized {
            window.maximize();
        }
        crate::wayland::set_app_id(&window, spec.id);

        let root = gtk::Overlay::new();
        root.set_overflow(gtk::Overflow::Hidden);
        let layer = gtk::Overlay::new();
        root.set_child(Some(&layer));
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("w11-frame");
        frame.set_size_request(opts.min_width, opts.min_height);
        layer.set_child(Some(&frame));

        // ---- title bar
        let titlebar = gtk::WindowHandle::new();
        titlebar.add_css_class("w11-titlebar");
        titlebar.set_size_request(-1, opts.titlebar_height.max(28));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.set_valign(gtk::Align::Fill);
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left.add_css_class("w11-titlebar-left");
        left.set_valign(gtk::Align::Center);
        let back = gtk::Button::new();
        back.add_css_class("w11-icon-button");
        back.add_css_class("w11-back");
        back.set_child(Some(&icons::glyph_stroke("arrow-left", 16, 1.4)));
        back.set_widget_name(&format!("{}-back", spec.kind));
        back.set_valign(gtk::Align::Center);
        back.set_sensitive(false);
        back.set_visible(opts.back);
        i18n::tooltip(&back, "Terug", "Back");
        let icon_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_slot.add_css_class("w11-appicon");
        icon_slot.set_valign(gtk::Align::Center);
        let title_label = gtk::Label::new(None);
        title_label.add_css_class("w11-apptitle");
        title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title_label.set_single_line_mode(true);
        title_label.set_valign(gtk::Align::Center);
        left.append(&back);
        left.append(&icon_slot);
        left.append(&title_label);
        let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        center.add_css_class("w11-titlebar-center");
        center.set_hexpand(true);
        center.set_valign(gtk::Align::Center);
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        end.set_valign(gtk::Align::Center);
        let captions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        captions.set_valign(gtk::Align::Start);
        let cap = |glyph: &str, name: &str, close: bool| {
            let b = gtk::Button::new();
            b.add_css_class("w11-cap");
            if close {
                b.add_css_class("w11-cap-close");
            }
            b.set_size_request(46, 32);
            b.set_valign(gtk::Align::Start);
            b.set_can_focus(false);
            b.set_focus_on_click(false);
            b.set_widget_name(&format!("{}-{name}", spec.kind));
            let g = icons::glyph(glyph, 10);
            g.add_css_class("w11-cap-glyph");
            b.set_child(Some(&g));
            (b, g)
        };
        let (min, _) = cap("cap-min", "minimize", false);
        i18n::tooltip(&min, "Minimaliseren", "Minimise");
        let (max, max_glyph) = cap("cap-max", "maximize", false);
        let (close, _) = cap("cap-close", "close", true);
        i18n::tooltip(&close, "Sluiten", "Close");
        captions.append(&min);
        captions.append(&max);
        captions.append(&close);
        bar.append(&left);
        bar.append(&center);
        bar.append(&end);
        bar.append(&captions);
        titlebar.set_child(Some(&bar));
        if opts.back {
            titlebar.add_css_class("w11-has-back");
        }
        frame.append(&titlebar);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("w11-content");
        content.set_vexpand(true);
        content.set_hexpand(true);
        frame.append(&content);

        // ---- toasts (bottom centre, over the content)
        let toasts = gtk::Box::new(gtk::Orientation::Vertical, 8);
        toasts.add_css_class("w11-toast-stack");
        toasts.set_halign(gtk::Align::Center);
        toasts.set_valign(gtk::Align::End);
        toasts.set_can_target(true);
        toasts.set_visible(false);
        layer.add_overlay(&toasts);

        let edges = edges(&root);
        window.set_child(Some(&root));

        let inner = Rc::new(Inner {
            window: window.clone(),
            kind: spec.kind,
            key: if opts.key.is_empty() { String::new() } else { key_of(spec.kind, &opts.key) },
            root,
            layer,
            frame,
            titlebar,
            titlebar_box: bar,
            back: back.clone(),
            icon_slot,
            title_label,
            left,
            title_widget: RefCell::new(None),
            center,
            end,
            max_button: max.clone(),
            max_glyph,
            content,
            toasts,
            edges,
            dialogs: RefCell::new(Vec::new()),
            shortcuts: RefCell::new(Vec::new()),
            on_back: RefCell::new(None),
            title_text: RefCell::new(String::new()),
        });
        WINDOWS.with(|w| {
            let mut w = w.borrow_mut();
            w.retain(|i| i.upgrade().is_some());
            w.push(Rc::downgrade(&inner));
        });
        let me = AppWindow { inner };
        let title = if opts.title.is_empty() { t(spec.title.0, spec.title.1).to_owned() } else { opts.title.clone() };
        me.set_title(&title);
        if opts.title.is_empty() {
            // The app's own name follows the language until the app sets a title of its own.
            let weak = Rc::downgrade(&me.inner);
            let initial = title.clone();
            let (nl, en) = spec.title;
            i18n::on_change(move || {
                if let Some(inner) = weak.upgrade() {
                    let cur = inner.title_text.borrow().clone();
                    if cur == initial || cur == nl || cur == en {
                        AppWindow { inner }.set_title(t(nl, en));
                    }
                }
            });
        }
        let icon = if opts.icon.is_empty() { spec.icon.to_owned() } else { opts.icon.clone() };
        me.set_icon(if icon == "-" { "" } else { &icon });

        // ---- behaviour
        let weak = me.downgrade();
        min.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.inner.window.minimize();
            }
        });
        let weak = me.downgrade();
        max.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.toggle_maximize();
            }
        });
        let weak = me.downgrade();
        close.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.inner.window.close();
            }
        });
        let weak = me.downgrade();
        back.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                let f = w.inner.on_back.borrow().clone();
                if let Some(f) = f {
                    f();
                }
            }
        });
        let weak = me.downgrade();
        let relabel = move || {
            if let Some(w) = weak.upgrade() {
                w.update_square();
            }
        };
        i18n::on_change(relabel.clone());
        let r = relabel.clone();
        window.connect_maximized_notify(move |_| r());
        let r = relabel.clone();
        window.connect_fullscreened_notify(move |_| r());
        let r = relabel.clone();
        window.connect_realize(move |w| {
            if let Some(surface) = w.surface()
                && let Ok(top) = surface.downcast::<gdk::Toplevel>()
            {
                let r = r.clone();
                top.connect_state_notify(move |_| r());
            }
        });
        let weak = me.downgrade();
        window.connect_close_request(move |_| {
            if let Some(w) = weak.upgrade() {
                w.save_size();
            }
            glib::Propagation::Proceed
        });
        me.install_keys();
        crate::backdrop::add_window(window.upcast_ref(), spec.kind);
        me.update_square();
        me
    }

    /// The AppWindow a widget is in (dialogs, toasts and menus find their window this way).
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Option<AppWindow> {
        let root = widget.as_ref().root()?;
        let window = root.downcast_ref::<gtk::Window>()?.clone();
        WINDOWS.with(|ws| {
            ws.borrow()
                .iter()
                .filter_map(|w| w.upgrade())
                .find(|i| i.window.upcast_ref::<gtk::Window>() == &window)
                .map(|inner| AppWindow { inner })
        })
    }

    /// The app's windows that are still open.
    pub fn all() -> Vec<AppWindow> {
        WINDOWS.with(|ws| ws.borrow().iter().filter_map(|w| w.upgrade()).map(|inner| AppWindow { inner }).collect())
    }

    /// The active window of the app, else the most recent one.
    pub fn active() -> Option<AppWindow> {
        let all = AppWindow::all();
        all.iter().find(|w| w.inner.window.is_active()).cloned().or_else(|| all.last().cloned())
    }

    fn downgrade(&self) -> WeakWindow {
        WeakWindow(Rc::downgrade(&self.inner))
    }

    /// The GTK window.
    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.inner.window
    }

    /// The title bar (a gtk::WindowHandle: whatever is not a button in it drags the window).
    pub fn titlebar(&self) -> &gtk::WindowHandle {
        &self.inner.titlebar
    }

    /// The layer over the whole frame where dialogs and toasts go (`add_overlay` for the app's own overlays).
    pub fn layer(&self) -> &gtk::Overlay {
        &self.inner.layer
    }

    /// Set the title (title bar text and GTK title).
    pub fn set_title(&self, title: &str) {
        *self.inner.title_text.borrow_mut() = title.to_owned();
        self.inner.title_label.set_label(title);
        self.inner.window.set_title(Some(title));
    }

    /// The title.
    pub fn title(&self) -> String {
        self.inner.title_text.borrow().clone()
    }

    /// Set the title bar icon: a glyph name (accent coloured, as appframe.js) or an app icon name; "" hides it.
    pub fn set_icon(&self, icon: &str) {
        let slot = &self.inner.icon_slot;
        while let Some(c) = slot.first_child() {
            slot.remove(&c);
        }
        if icon.is_empty() {
            slot.set_visible(false);
            self.inner.titlebar.add_css_class("w11-no-icon");
            return;
        }
        slot.set_visible(true);
        self.inner.titlebar.remove_css_class("w11-no-icon");
        if icons::has_glyph(icon) {
            slot.append(&icons::glyph(icon, 16));
        } else {
            slot.append(&icons::app_icon(icon, 16));
        }
    }

    /// Replace the icon and title with the app's own widget (None: back to icon and title).
    pub fn set_title_widget(&self, widget: Option<&impl IsA<gtk::Widget>>) {
        if let Some(old) = self.inner.title_widget.borrow_mut().take() {
            self.inner.left.remove(&old);
        }
        let has = widget.is_some();
        self.inner.icon_slot.set_visible(!has && self.inner.icon_slot.first_child().is_some());
        self.inner.title_label.set_visible(!has);
        if let Some(w) = widget {
            let w = w.as_ref().clone();
            self.inner.left.append(&w);
            *self.inner.title_widget.borrow_mut() = Some(w);
        }
    }

    /// The app's content in the title bar's middle (it expands; a search box, Explorer's tabs). None clears it.
    pub fn set_titlebar_content(&self, widget: Option<&impl IsA<gtk::Widget>>) {
        let c = &self.inner.center;
        while let Some(child) = c.first_child() {
            c.remove(&child);
        }
        if let Some(w) = widget {
            c.append(w.as_ref());
        }
    }

    /// Add a widget just before the caption buttons.
    pub fn add_titlebar_end(&self, widget: &impl IsA<gtk::Widget>) {
        self.inner.end.append(widget.as_ref());
    }

    /// The title bar's row (back, icon and title, centre, end, captions) for custom layouts.
    pub fn titlebar_box(&self) -> &gtk::Box {
        &self.inner.titlebar_box
    }

    /// Set the window's content (below the title bar).
    pub fn set_content(&self, widget: &impl IsA<gtk::Widget>) {
        let c = &self.inner.content;
        while let Some(child) = c.first_child() {
            c.remove(&child);
        }
        let w = widget.as_ref();
        w.set_vexpand(true);
        c.append(w);
    }

    /// The content box (vertical, expands).
    pub fn content(&self) -> &gtk::Box {
        &self.inner.content
    }

    /// Show the Back button.
    pub fn set_back_visible(&self, visible: bool) {
        self.inner.back.set_visible(visible);
        if visible {
            self.inner.titlebar.add_css_class("w11-has-back");
        } else {
            self.inner.titlebar.remove_css_class("w11-has-back");
        }
    }

    /// Enable or disable the Back button.
    pub fn set_back_enabled(&self, enabled: bool) {
        self.inner.back.set_sensitive(enabled);
    }

    /// What Back (and Alt+Left) does.
    pub fn connect_back(&self, f: impl Fn() + 'static) {
        *self.inner.on_back.borrow_mut() = Some(Rc::new(f));
    }

    /// A keyboard shortcut for this window ("<Control>f", "F5", "<Alt>Left", "Delete"); `f` returns whether it
    /// handled the key. Not while a dialog is open. The dev channel's "key" command uses these too.
    pub fn add_shortcut(&self, accel: &str, f: impl Fn() -> bool + 'static) {
        let Some((key, mods)) = gtk::accelerator_parse(accel) else {
            eprintln!("win11-kit: bad accelerator {accel}");
            return;
        };
        self.inner.shortcuts.borrow_mut().push(Shortcut { accel: accel.to_owned(), key, mods, f: Rc::new(f) });
    }

    /// Run the shortcut registered for this accelerator (the dev channel); true when one handled it.
    pub fn run_shortcut(&self, accel: &str) -> bool {
        let Some((key, mods)) = gtk::accelerator_parse(accel) else { return false };
        self.dispatch_key(key, mods)
    }

    fn dispatch_key(&self, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        let mask = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        let mods = mods & mask;
        let lower = key.to_lower();
        let found: Vec<Rc<dyn Fn() -> bool>> = self
            .inner
            .shortcuts
            .borrow()
            .iter()
            .filter(|s| s.key.to_lower() == lower && (s.mods & mask) == mods)
            .map(|s| s.f.clone())
            .collect();
        found.into_iter().any(|f| f())
    }

    /// The accelerators registered with `add_shortcut`.
    pub fn shortcuts(&self) -> Vec<String> {
        self.inner.shortcuts.borrow().iter().map(|s| s.accel.clone()).collect()
    }

    fn install_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(w) = weak.upgrade() else { return glib::Propagation::Proceed };
            if w.dialog_open() {
                return glib::Propagation::Proceed;
            }
            if key == gdk::Key::Left && state.contains(gdk::ModifierType::ALT_MASK) && w.inner.back.is_visible() && w.inner.back.is_sensitive() {
                let f = w.inner.on_back.borrow().clone();
                if let Some(f) = f {
                    f();
                    return glib::Propagation::Stop;
                }
            }
            if w.dispatch_key(key, state) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.inner.window.add_controller(keys);
    }

    /// Present the window and give it the focus.
    pub fn present(&self) {
        self.inner.window.present();
    }

    /// Close the window.
    pub fn close(&self) {
        self.inner.window.close();
    }

    /// Maximize or restore.
    pub fn toggle_maximize(&self) {
        if self.inner.window.is_maximized() {
            self.inner.window.unmaximize();
        } else {
            self.inner.window.maximize();
        }
    }

    /// Remember the size now (it also happens when the window closes).
    pub fn save_size(&self) {
        if self.inner.key.is_empty() {
            return;
        }
        let (w, h) = self.inner.window.default_size();
        let maximized = self.inner.window.is_maximized();
        if w > 0 && h > 0 {
            save_size(&self.inner.key, w, h, maximized);
        }
    }

    // ------------------------------------------------------------------------------------- dialogs layer

    /// Whether a dialog is open in this window.
    pub fn dialog_open(&self) -> bool {
        !self.inner.dialogs.borrow().is_empty()
    }

    /// Put a modal layer (smoke and dialog) over the window; the frame below takes no focus meanwhile.
    pub(crate) fn push_dialog(&self, layer: &gtk::Widget) {
        self.inner.layer.add_overlay(layer);
        self.inner.frame.set_can_focus(false);
        self.inner.dialogs.borrow_mut().push(layer.clone());
    }

    /// Take a dialog layer away again.
    pub(crate) fn pop_dialog(&self, layer: &gtk::Widget) {
        self.inner.dialogs.borrow_mut().retain(|l| l != layer);
        if layer.parent().as_ref() == Some(self.inner.layer.upcast_ref::<gtk::Widget>()) {
            self.inner.layer.remove_overlay(layer);
        }
        if self.inner.dialogs.borrow().is_empty() {
            self.inner.frame.set_can_focus(true);
        }
    }

    /// The topmost dialog layer.
    pub(crate) fn top_dialog(&self) -> Option<gtk::Widget> {
        self.inner.dialogs.borrow().last().cloned()
    }

    pub(crate) fn toast_stack(&self) -> &gtk::Box {
        &self.inner.toasts
    }

    /// Square corners and no resize edges while maximized, fullscreen or tiled; the caption button follows.
    fn update_square(&self) {
        let w = &self.inner.window;
        let mut square = w.is_maximized() || w.is_fullscreen();
        let mut fixed = square;
        if let Some(surface) = w.surface()
            && let Ok(top) = surface.downcast::<gdk::Toplevel>()
        {
            let s = top.state();
            let tiled = gdk::ToplevelState::TILED
                | gdk::ToplevelState::TOP_TILED
                | gdk::ToplevelState::RIGHT_TILED
                | gdk::ToplevelState::BOTTOM_TILED
                | gdk::ToplevelState::LEFT_TILED;
            if s.intersects(tiled) {
                square = true;
            }
            if s.intersects(gdk::ToplevelState::MAXIMIZED | gdk::ToplevelState::FULLSCREEN) {
                square = true;
                fixed = true;
            }
        }
        if square {
            w.add_css_class("w11-square");
        } else {
            w.remove_css_class("w11-square");
        }
        for e in &self.inner.edges {
            e.set_visible(!fixed);
        }
        let maximized = w.is_maximized();
        icons::set_glyph(&self.inner.max_glyph, if maximized { "cap-restore" } else { "cap-max" });
        self.inner.max_button.set_tooltip_text(Some(if maximized {
            t("Vorige grootte", "Restore down")
        } else {
            t("Maximaliseren", "Maximise")
        }));
    }

    /// The app kind this window belongs to.
    pub fn kind(&self) -> &'static str {
        self.inner.kind
    }

    /// The root overlay (edges on top; the layer below them).
    pub fn root(&self) -> &gtk::Overlay {
        &self.inner.root
    }
}

struct WeakWindow(Weak<Inner>);

impl WeakWindow {
    fn upgrade(&self) -> Option<AppWindow> {
        self.0.upgrade().map(|inner| AppWindow { inner })
    }
}

impl Clone for WeakWindow {
    fn clone(&self) -> WeakWindow {
        WeakWindow(self.0.clone())
    }
}

/// Invisible strips along the window's edges and corners that start a compositor resize.
fn edges(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    use gdk::SurfaceEdge as E;
    use gtk::Align::{End, Fill, Start};
    let specs = [
        (E::North, Fill, Start, -1, 4, "n-resize"),
        (E::South, Fill, End, -1, 5, "s-resize"),
        (E::West, Start, Fill, 5, -1, "w-resize"),
        (E::East, End, Fill, 5, -1, "e-resize"),
        (E::NorthWest, Start, Start, 10, 10, "nw-resize"),
        (E::NorthEast, End, Start, 10, 6, "ne-resize"),
        (E::SouthWest, Start, End, 12, 12, "sw-resize"),
        (E::SouthEast, End, End, 12, 12, "se-resize"),
    ];
    let mut out = Vec::new();
    for (edge, h, v, width, height, cursor) in specs {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        b.add_css_class("w11-edge");
        b.set_halign(h);
        b.set_valign(v);
        b.set_size_request(width, height);
        b.set_cursor_from_name(Some(cursor));
        let g = gtk::GestureClick::new();
        g.set_button(1);
        g.connect_pressed(move |g, _, x, y| begin_resize(g, edge, x, y));
        b.add_controller(g);
        overlay.add_overlay(&b);
        out.push(b.upcast());
    }
    out
}

fn begin_resize(g: &gtk::GestureClick, edge: gdk::SurfaceEdge, x: f64, y: f64) {
    let Some(w) = g.widget() else { return };
    let Some(native) = w.native() else { return };
    let Some(surface) = native.surface() else { return };
    let Ok(top) = surface.downcast::<gdk::Toplevel>() else { return };
    let Some(p) = w.compute_point(native.upcast_ref::<gtk::Widget>(), &graphene::Point::new(x as f32, y as f32)) else {
        return;
    };
    let (tx, ty) = native.surface_transform();
    g.set_state(gtk::EventSequenceState::Claimed);
    let device = g.device();
    top.begin_resize(edge, device.as_ref(), 1, p.x() as f64 + tx, p.y() as f64 + ty, g.current_event_time());
}

// ---------------------------------------------------------------------------------------- remembered size

fn key_of(kind: &str, key: &str) -> String {
    format!("{kind}:{key}")
}

fn sizes_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(glib::user_config_dir);
    base.join("win11-gnome").join("native-windows.json")
}

fn read_sizes() -> Value {
    std::fs::read(sizes_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn load_size(key: &str) -> Option<(i32, i32, bool)> {
    let v = read_sizes();
    let e = v.get(key)?;
    let w = e["width"].as_i64()? as i32;
    let h = e["height"].as_i64()? as i32;
    (w > 100 && h > 100 && w < 20000 && h < 20000).then_some((w, h, e["maximized"] == true))
}

fn save_size(key: &str, w: i32, h: i32, maximized: bool) {
    let mut v = read_sizes();
    if let Some(obj) = v.as_object_mut() {
        obj.insert(key.to_owned(), json!({ "width": w, "height": h, "maximized": maximized }));
    }
    let path = sizes_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
