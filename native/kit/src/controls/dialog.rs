//! The content dialog (components.js Dialog, WinUI ContentDialog): modal over the window on a 30 % smoke,
//! title (with an optional glyph or severity disc), a body (text or any widget), and primary / secondary /
//! close buttons of equal width (primary accent, or danger red); Enter runs the default button, Escape closes.
//! While busy (`set_busy`, or while `on_primary` keeps it open) Enter, Escape and the buttons do nothing.
use crate::backend::CallError;
use crate::frame::AppWindow;
use crate::i18n::t;
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Which button closed a dialog (Escape is Close).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogResult {
    Primary,
    Secondary,
    Close,
}

type Done = Box<dyn FnOnce(DialogResult)>;

struct Inner {
    title: RefCell<String>,
    body: RefCell<Option<gtk::Widget>>,
    icon: RefCell<String>,
    primary: RefCell<Option<String>>,
    secondary: RefCell<Option<String>>,
    close: RefCell<Option<Option<String>>>,
    danger: Cell<bool>,
    default: Cell<Option<DialogResult>>,
    width: Cell<i32>,
    on_primary: RefCell<Option<Rc<dyn Fn(&Dialog) -> bool>>>,
    busy: Cell<bool>,
    done: RefCell<Option<Done>>,
    layer: RefCell<Option<gtk::Widget>>,
    window: RefCell<Option<AppWindow>>,
    buttons: RefCell<Vec<(DialogResult, gtk::Button)>>,
    focus_before: RefCell<Option<gtk::Widget>>,
}

/// A content dialog, built by chaining and shown with `show`.
#[derive(Clone)]
pub struct Dialog {
    inner: Rc<Inner>,
}

impl Dialog {
    pub fn new(title: &str) -> Dialog {
        Dialog {
            inner: Rc::new(Inner {
                title: RefCell::new(title.to_owned()),
                body: RefCell::new(None),
                icon: RefCell::new(String::new()),
                primary: RefCell::new(None),
                secondary: RefCell::new(None),
                close: RefCell::new(None),
                danger: Cell::new(false),
                default: Cell::new(None),
                width: Cell::new(0),
                on_primary: RefCell::new(None),
                busy: Cell::new(false),
                done: RefCell::new(None),
                layer: RefCell::new(None),
                window: RefCell::new(None),
                buttons: RefCell::new(Vec::new()),
                focus_before: RefCell::new(None),
            }),
        }
    }

    /// The body as text (wrapping, line breaks kept).
    pub fn text(self, text: &str) -> Dialog {
        let l = super::label(text, "w11-dialog-text", true);
        l.set_max_width_chars(62);
        l.set_selectable(false);
        *self.inner.body.borrow_mut() = Some(l.upcast());
        self
    }

    /// Any widget as the body.
    pub fn body(self, w: &impl IsA<gtk::Widget>) -> Dialog {
        *self.inner.body.borrow_mut() = Some(w.as_ref().clone());
        self
    }

    /// A glyph before the title; "info", "success", "warning" or "error" give the severity disc.
    pub fn icon(self, glyph: &str) -> Dialog {
        *self.inner.icon.borrow_mut() = glyph.to_owned();
        self
    }

    pub fn primary(self, label: &str) -> Dialog {
        *self.inner.primary.borrow_mut() = Some(label.to_owned());
        self
    }

    pub fn secondary(self, label: &str) -> Dialog {
        *self.inner.secondary.borrow_mut() = Some(label.to_owned());
        self
    }

    /// The close button's text (default "Annuleren" with a primary or secondary button, else "Sluiten").
    pub fn close_label(self, label: &str) -> Dialog {
        *self.inner.close.borrow_mut() = Some(Some(label.to_owned()));
        self
    }

    /// No close button.
    pub fn no_close(self) -> Dialog {
        *self.inner.close.borrow_mut() = Some(None);
        self
    }

    /// The primary button is red (a destructive action); the default button becomes Close.
    pub fn danger(self) -> Dialog {
        self.inner.danger.set(true);
        self
    }

    /// The button Enter runs (default: Primary, or Close for a danger dialog).
    pub fn default_button(self, r: DialogResult) -> Dialog {
        self.inner.default.set(Some(r));
        self
    }

    /// A fixed width in px (default: by content, 320 at least).
    pub fn width(self, px: i32) -> Dialog {
        self.inner.width.set(px);
        self
    }

    /// Runs on Primary before closing; false keeps the dialog open (validation, or work that ends with
    /// `close(DialogResult::Primary)` later).
    pub fn on_primary(self, f: impl Fn(&Dialog) -> bool + 'static) -> Dialog {
        *self.inner.on_primary.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Show it over the window `parent` is in; `done(result)` runs once it closes. Without an AppWindow the
    /// dialog closes at once with Close.
    pub fn show(&self, parent: &impl IsA<gtk::Widget>, done: impl FnOnce(DialogResult) + 'static) {
        let window = AppWindow::of(parent).or_else(AppWindow::active);
        let Some(window) = window else {
            done(DialogResult::Close);
            return;
        };
        *self.inner.done.borrow_mut() = Some(Box::new(done));
        *self.inner.focus_before.borrow_mut() = GtkWindowExt::focus(window.window());
        let smoke = gtk::Box::new(gtk::Orientation::Vertical, 0);
        smoke.add_css_class("w11-smoke");
        smoke.set_hexpand(true);
        smoke.set_vexpand(true);
        smoke.set_can_target(true);
        smoke.set_widget_name("w11-dialog");
        // The layer owns the dialog until it closes (the caller may drop its handle after show).
        super::own(&smoke, &self.inner);
        let dialog = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dialog.add_css_class("w11-dialog");
        dialog.set_halign(gtk::Align::Center);
        dialog.set_valign(gtk::Align::Center);
        dialog.set_vexpand(true);
        dialog.set_margin_start(24);
        dialog.set_margin_end(24);
        dialog.set_margin_top(24);
        dialog.set_margin_bottom(24);
        let w = self.inner.width.get();
        dialog.set_size_request(if w > 0 { w } else { 320 }, -1);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("w11-dialog-body");
        let title = self.inner.title.borrow().clone();
        if !title.is_empty() {
            let tb = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            tb.add_css_class("w11-dialog-titlebox");
            tb.add_css_class("w11-dialog-title");
            let icon = self.inner.icon.borrow().clone();
            if matches!(icon.as_str(), "info" | "success" | "warning" | "error") {
                tb.append(&icons::severity_icon(&icon, 20));
            } else if !icon.is_empty() {
                tb.append(&icons::glyph(&icon, 20));
            }
            let tl = super::label(&title, "", true);
            tl.set_max_width_chars(50);
            tb.append(&tl);
            body.append(&tb);
        }
        if let Some(b) = self.inner.body.borrow().as_ref() {
            body.append(b);
        }
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("w11-dialog-footer");
        footer.set_homogeneous(true);
        let primary = self.inner.primary.borrow().clone();
        let secondary = self.inner.secondary.borrow().clone();
        let close = match self.inner.close.borrow().clone() {
            Some(c) => c,
            None => Some(if primary.is_some() || secondary.is_some() { t("Annuleren", "Cancel") } else { t("Sluiten", "Close") }.to_owned()),
        };
        let mut buttons = Vec::new();
        let danger = self.inner.danger.get();
        if let Some(p) = &primary {
            let b = super::buttons::button(p, if danger { super::Variant::Danger } else { super::Variant::Accent });
            b.set_widget_name("w11-dialog-primary");
            buttons.push((DialogResult::Primary, b));
        }
        if let Some(s) = &secondary {
            let b = super::buttons::button(s, super::Variant::Standard);
            b.set_widget_name("w11-dialog-secondary");
            buttons.push((DialogResult::Secondary, b));
        }
        if let Some(c) = &close {
            let only = primary.is_none() && secondary.is_none();
            let b = super::buttons::button(c, if only { super::Variant::Accent } else { super::Variant::Standard });
            b.set_widget_name("w11-dialog-close");
            buttons.push((DialogResult::Close, b));
        }
        for (r, b) in &buttons {
            b.set_hexpand(true);
            let r = *r;
            let weak = Rc::downgrade(&self.inner);
            b.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    Dialog { inner }.finish(r, false);
                }
            });
            footer.append(b);
        }
        dialog.append(&body);
        dialog.append(&footer);
        smoke.append(&dialog);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
            let me = Dialog { inner };
            match key {
                gdk::Key::Escape => {
                    if !me.inner.busy.get() {
                        me.finish(DialogResult::Close, false);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if me.inner.busy.get() {
                        return glib::Propagation::Stop;
                    }
                    let focus = me.inner.window.borrow().as_ref().and_then(|w| GtkWindowExt::focus(w.window()));
                    let skip = focus.as_ref().is_some_and(|f| {
                        f.is::<gtk::TextView>() || f.is::<gtk::Button>() || f.ancestor(gtk::Button::static_type()).is_some()
                    });
                    if skip {
                        return glib::Propagation::Proceed;
                    }
                    me.finish(me.default_result(), false);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        smoke.add_controller(keys);
        // Clicks on the smoke stay there (modal).
        let block = gtk::GestureClick::new();
        block.connect_pressed(|g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        smoke.add_controller(block);
        window.push_dialog(smoke.upcast_ref());
        *self.inner.layer.borrow_mut() = Some(smoke.upcast());
        *self.inner.window.borrow_mut() = Some(window);
        *self.inner.buttons.borrow_mut() = buttons;
        let weak = Rc::downgrade(&self.inner);
        glib::idle_add_local_once(move || {
            let Some(inner) = weak.upgrade() else { return };
            let me = Dialog { inner };
            let entry = me.inner.body.borrow().as_ref().and_then(first_entry);
            if let Some(e) = entry {
                e.grab_focus();
                return;
            }
            let def = me.default_result();
            let b = me.inner.buttons.borrow().iter().find(|(r, _)| *r == def).map(|(_, b)| b.clone());
            if let Some(b) = b.or_else(|| me.inner.buttons.borrow().first().map(|(_, b)| b.clone())) {
                b.grab_focus();
            }
        });
    }

    fn default_result(&self) -> DialogResult {
        if let Some(d) = self.inner.default.get() {
            return d;
        }
        if self.inner.danger.get() {
            DialogResult::Close
        } else if self.inner.primary.borrow().is_some() {
            DialogResult::Primary
        } else if self.inner.secondary.borrow().is_some() {
            DialogResult::Secondary
        } else {
            DialogResult::Close
        }
    }

    fn finish(&self, r: DialogResult, force: bool) {
        if self.inner.busy.get() && !force {
            return;
        }
        if r == DialogResult::Primary && !force {
            let f = self.inner.on_primary.borrow().clone();
            if let Some(f) = f
                && !f(self)
            {
                return;
            }
        }
        let Some(layer) = self.inner.layer.borrow_mut().take() else { return };
        if let Some(w) = self.inner.window.borrow().as_ref() {
            w.pop_dialog(&layer);
        }
        if let Some(f) = self.inner.focus_before.borrow_mut().take()
            && f.is_mapped()
        {
            f.grab_focus();
        }
        let done = self.inner.done.borrow_mut().take();
        if let Some(done) = done {
            done(r);
        }
    }

    /// Close it from code (also while busy).
    pub fn close(&self, r: DialogResult) {
        self.finish(r, true);
    }

    /// Busy: buttons, Enter and Escape do nothing until `set_busy(false)` or `close`.
    pub fn set_busy(&self, busy: bool) {
        self.inner.busy.set(busy);
        for (_, b) in self.inner.buttons.borrow().iter() {
            b.set_sensitive(!busy);
        }
    }

    /// Enable or disable the primary button (e.g. until a name is typed).
    pub fn set_primary_enabled(&self, enabled: bool) {
        for (r, b) in self.inner.buttons.borrow().iter() {
            if *r == DialogResult::Primary {
                b.set_sensitive(enabled);
            }
        }
    }

    /// Whether it is showing.
    pub fn is_open(&self) -> bool {
        self.inner.layer.borrow().is_some()
    }
}

fn first_entry(w: &gtk::Widget) -> Option<gtk::Widget> {
    if (w.is::<gtk::Entry>() || w.is::<gtk::Text>() || w.is::<gtk::TextView>()) && w.is_sensitive() {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(f) = first_entry(&child) {
            return Some(f);
        }
        c = child.next_sibling();
    }
    None
}

/// Dialog.confirm: a question with a confirm button (red with `danger`) and Annuleren; `done(true)` when confirmed.
pub fn confirm(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    message: &str,
    confirm_label: &str,
    danger: bool,
    done: impl FnOnce(bool) + 'static,
) {
    let mut d = Dialog::new(title).text(message).primary(if confirm_label.is_empty() { "OK" } else { confirm_label });
    if danger {
        d = d.danger();
    }
    d.show(parent, move |r| done(r == DialogResult::Primary));
}

/// Dialog.alert: a message with one OK button.
pub fn alert(parent: &impl IsA<gtk::Widget>, title: &str, message: &str, done: impl FnOnce() + 'static) {
    Dialog::new(title).text(message).close_label("OK").show(parent, move |_| done());
}

/// "Deze snelkoppeling starten?" for an untrusted .desktop launcher (bridge.js confirmLauncher): show what it
/// runs, and only after "Starten toestaan" mark it trusted (files.trustLauncher) and run the call once more.
pub(crate) fn confirm_launcher(
    err: CallError,
    method: String,
    args: Value,
    done: Box<dyn FnOnce(Result<Value, CallError>)>,
) {
    let Some(window) = AppWindow::active() else {
        done(Err(err));
        return;
    };
    let d = &err.data;
    let path = d["path"].as_str().unwrap_or("").to_owned();
    let name = d["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("").to_owned());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let intro = if crate::i18n::en() {
        format!("\u{201c}{name}\u{201d} is a shortcut that starts a program. Only allow it if you trust where it came from.")
    } else {
        format!("\u{2018}{name}\u{2019} is een snelkoppeling die een programma start. Sta dit alleen toe als je weet waar hij vandaan komt.")
    };
    let l = super::label(&intro, "w11-dialog-text", true);
    l.set_max_width_chars(62);
    body.append(&l);
    let runs = super::label(t("Dit wordt uitgevoerd:", "This will run:"), "w11-caption w11-text-2", false);
    runs.set_margin_top(4);
    body.append(&runs);
    let cmd = super::label(d["exec"].as_str().unwrap_or("?"), "w11-mono w11-launcher-cmd", true);
    cmd.set_selectable(true);
    cmd.set_max_width_chars(62);
    body.append(&cmd);
    body.append(&super::label(&path, "w11-caption w11-text-3", true));
    let done = Rc::new(RefCell::new(Some(done)));
    Dialog::new(t("Deze snelkoppeling starten?", "Start this shortcut?"))
        .body(&body)
        .icon("shield")
        .primary(t("Starten toestaan", "Allow launching"))
        .close_label(t("Annuleren", "Cancel"))
        .default_button(DialogResult::Close)
        .show(window.window(), move |r| {
            let Some(done) = done.borrow_mut().take() else { return };
            if r != DialogResult::Primary {
                done(Err(CallError::new("cancelled", t("Niet gestart.", "Not started."))));
                return;
            }
            let Some(ctx) = crate::ctx::try_ctx() else { return };
            let backend = ctx.backend();
            crate::ctx::spawn(
                move || backend.call("files.trustLauncher", &json!({ "path": path })).and_then(|_| backend.call(&method, &args)),
                done,
            );
        });
}
