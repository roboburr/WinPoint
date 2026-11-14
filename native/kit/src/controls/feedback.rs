//! Info bar and in-app toast (components.js InfoBar, Toast).
use crate::frame::AppWindow;
use crate::i18n;
use crate::icons;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Info bar and toast severities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Success => "success",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
    pub fn parse(s: &str) -> Severity {
        match s {
            "success" => Severity::Success,
            "warning" | "caution" => Severity::Warning,
            "error" | "critical" => Severity::Error,
            _ => Severity::Info,
        }
    }
}

// ------------------------------------------------------------------------------------- the info bar's body

/// Between the text and an action on its line (components.css `.fl-infobar__action` margin-left).
const ACTION_GAP: i32 = 12;
/// Between the text and an action under it.
const ACTION_GAP_BELOW: i32 = 8;
/// How far an action on the text's line reaches into the body's padding above and below (the HTML's
/// `margin: -6px 0`), so a 32 px button keeps the bar one text line high.
const ACTION_OVERHANG: i32 = 6;

mod body_imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub(crate) struct InfoBody {
        pub text: RefCell<Option<gtk::Label>>,
        pub action: RefCell<Option<gtk::Box>>,
        pub below: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InfoBody {
        const NAME: &'static str = "W11InfoBarBody";
        type Type = super::InfoBody;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for InfoBody {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for InfoBody {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (min, nat) = self.obj().measure_body(orientation, for_size);
            (min, nat, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let p = self.obj().place(width);
            if let (Some(l), Some(r)) = (self.text.borrow().as_ref(), p.text) {
                l.size_allocate(&gtk::Allocation::new(r.0, r.1, r.2, r.3), -1);
            }
            if let (Some(a), Some(r)) = (self.action.borrow().as_ref(), p.action) {
                a.size_allocate(&gtk::Allocation::new(r.0, r.1, r.2, r.3), -1);
            }
        }
    }
}

glib::wrapper! {
    /// The text and the action of an info bar. Title and message are one wrapping label (as the HTML's inline
    /// spans), so a long message flows on under the title over the bar's whole width. The action sits at the
    /// end of the text's line while text and action fit on one line, and under the text when they do not.
    pub(crate) struct InfoBody(ObjectSubclass<body_imp::InfoBody>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Where `InfoBody` puts its children for a width: (x, y, width, height) each, and its own height.
struct Placed {
    text: Option<(i32, i32, i32, i32)>,
    action: Option<(i32, i32, i32, i32)>,
    height: i32,
}

impl InfoBody {
    fn new(text: &gtk::Label, action: &gtk::Box) -> InfoBody {
        let body: InfoBody = glib::Object::new();
        text.set_parent(&body);
        action.set_parent(&body);
        *body.imp().text.borrow_mut() = Some(text.clone());
        *body.imp().action.borrow_mut() = Some(action.clone());
        body
    }

    fn set_below(&self, below: bool) {
        if self.imp().below.replace(below) != below {
            self.queue_resize();
        }
    }

    /// The shown children with their (minimum, natural) widths.
    fn parts(&self) -> (Option<(gtk::Label, i32, i32)>, Option<(gtk::Box, i32, i32)>) {
        let imp = self.imp();
        let text = imp.text.borrow().clone().filter(|l| l.is_visible()).map(|l| {
            let (min, nat, _, _) = l.measure(gtk::Orientation::Horizontal, -1);
            (l, min, nat)
        });
        let action = imp.action.borrow().clone().filter(|a| a.is_visible()).map(|a| {
            let (min, nat, _, _) = a.measure(gtk::Orientation::Horizontal, -1);
            (a, min, nat)
        });
        (text, action)
    }

    fn measure_body(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32) {
        let (text, action) = self.parts();
        let (tmin, tnat) = text.as_ref().map_or((0, 0), |t| (t.1, t.2));
        let (amin, anat) = action.as_ref().map_or((0, 0), |a| (a.1, a.2));
        let nat_w = match (&text, &action) {
            (Some(_), Some(_)) if !self.imp().below.get() => tnat + ACTION_GAP + anat,
            _ => tnat.max(anat),
        };
        if orientation == gtk::Orientation::Horizontal {
            return (tmin.max(amin), nat_w);
        }
        let h = self.place(if for_size >= 0 { for_size } else { nat_w }).height;
        (h, h)
    }

    fn place(&self, width: i32) -> Placed {
        let (text, action) = self.parts();
        let below = self.imp().below.get();
        let height_for = |w: &gtk::Widget, width: i32| w.measure(gtk::Orientation::Vertical, width).1;
        match (text, action) {
            (None, None) => Placed { text: None, action: None, height: 0 },
            (Some((l, tmin, _)), None) => {
                let w = width.max(tmin);
                let h = height_for(l.upcast_ref(), w);
                Placed { text: Some((0, 0, w, h)), action: None, height: h }
            }
            (None, Some((a, amin, anat))) => {
                let w = anat.min(width).max(amin);
                let h = height_for(a.upcast_ref(), w);
                Placed { text: None, action: Some((0, 0, w, h)), height: h }
            }
            (Some((l, tmin, tnat)), Some((a, amin, anat))) => {
                if !below && tnat + ACTION_GAP + anat <= width {
                    let th = height_for(l.upcast_ref(), tnat);
                    let ah = height_for(a.upcast_ref(), anat);
                    let h = th.max(ah - 2 * ACTION_OVERHANG);
                    Placed {
                        text: Some((0, (h - th) / 2, tnat, th)),
                        action: Some((tnat + ACTION_GAP, (h - ah) / 2, anat, ah)),
                        height: h,
                    }
                } else {
                    let w = width.max(tmin);
                    let th = height_for(l.upcast_ref(), w);
                    let aw = anat.min(width).max(amin);
                    let ah = height_for(a.upcast_ref(), aw);
                    Placed {
                        text: Some((0, 0, w, th)),
                        action: Some((0, th + ACTION_GAP_BELOW, aw, ah)),
                        height: th + ACTION_GAP_BELOW + ah,
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- info bar

struct BarInner {
    root: glib::WeakRef<gtk::Box>,
    icon: gtk::Box,
    body: InfoBody,
    text: gtk::Label,
    title: RefCell<String>,
    message: RefCell<String>,
    action: gtk::Box,
    close: gtk::Button,
    severity: Cell<Severity>,
    on_close: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl BarInner {
    /// Title (semibold) and message as one text, as the HTML's inline spans: they wrap together.
    fn render_text(&self) {
        let (title, message) = (self.title.borrow(), self.message.borrow());
        let markup = match (title.is_empty(), message.is_empty()) {
            (true, _) => glib::markup_escape_text(&message).to_string(),
            (false, true) => format!("<span weight=\"600\">{}</span>", glib::markup_escape_text(&title)),
            // an em space: the title's 12 px margin (components.css), and a place to wrap
            (false, false) => format!(
                "<span weight=\"600\">{}</span>\u{2003}{}",
                glib::markup_escape_text(&title),
                glib::markup_escape_text(&message)
            ),
        };
        self.text.set_markup(&markup);
        self.text.set_visible(!title.is_empty() || !message.is_empty());
    }
}

/// An info bar (`.fl-infobar`): severity disc, semibold title and message flowing as one text, an optional
/// action (at the end of the line when it fits, else under the text) and a close button. Hidden with
/// `set_visible(false)` / shown with `show()`; closing hides it and runs `connect_closed`.
#[derive(Clone)]
pub struct InfoBar {
    root: gtk::Box,
    inner: Rc<BarInner>,
}

impl InfoBar {
    pub fn new(severity: Severity, title: &str, message: &str) -> InfoBar {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("w11-infobar");
        let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon.add_css_class("w11-infobar-icon");
        icon.set_valign(gtk::Align::Start);
        let text = super::label("", "w11-infobar-text w11-infobar-message", true);
        let action = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        action.add_css_class("w11-infobar-action");
        action.set_visible(false);
        let body = InfoBody::new(&text, &action);
        body.add_css_class("w11-infobar-body");
        body.set_hexpand(true);
        let close = super::buttons::icon_button("close", "", 38);
        close.add_css_class("w11-infobar-close");
        close.set_valign(gtk::Align::Start);
        i18n::tooltip(&close, "Sluiten", "Close");
        root.append(&icon);
        root.append(&body);
        root.append(&close);
        let inner = Rc::new(BarInner {
            root: root.downgrade(),
            icon,
            body,
            text,
            title: RefCell::new(title.to_owned()),
            message: RefCell::new(message.to_owned()),
            action,
            close: close.clone(),
            severity: Cell::new(severity),
            on_close: RefCell::new(Vec::new()),
        });
        super::own(&root, &inner);
        let me = InfoBar { root, inner };
        me.inner.render_text();
        me.set_severity(severity);
        let weak = Rc::downgrade(&me.inner);
        close.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                if let Some(root) = inner.root.upgrade() {
                    root.set_visible(false);
                }
                let hs: Vec<Rc<dyn Fn()>> = inner.on_close.borrow().clone();
                for h in hs {
                    h();
                }
            }
        });
        me
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_title(&self, s: &str) {
        *self.inner.title.borrow_mut() = s.to_owned();
        self.inner.render_text();
    }

    pub fn set_message(&self, s: &str) {
        *self.inner.message.borrow_mut() = s.to_owned();
        self.inner.render_text();
    }

    pub fn set_severity(&self, s: Severity) {
        let root = &self.root;
        for c in ["w11-success", "w11-warning", "w11-error"] {
            root.remove_css_class(c);
        }
        match s {
            Severity::Success => root.add_css_class("w11-success"),
            Severity::Warning => root.add_css_class("w11-warning"),
            Severity::Error => root.add_css_class("w11-error"),
            Severity::Info => {}
        }
        self.inner.severity.set(s);
        super::misc::clear(&self.inner.icon);
        self.inner.icon.append(&icons::severity_icon(s.as_str(), 16));
    }

    pub fn severity(&self) -> Severity {
        self.inner.severity.get()
    }

    /// A standard button as the action (None removes it).
    pub fn set_action(&self, action: Option<(&str, Rc<dyn Fn()>)>) {
        super::misc::clear(&self.inner.action);
        match action {
            Some((label, f)) => {
                let b = super::buttons::button(label, super::Variant::Standard);
                b.connect_clicked(move |_| f());
                self.inner.action.append(&b);
                self.inner.action.set_visible(true);
            }
            None => self.inner.action.set_visible(false),
        }
    }

    /// Any widget as the action.
    pub fn set_action_widget(&self, w: &impl IsA<gtk::Widget>) {
        super::misc::clear(&self.inner.action);
        self.inner.action.append(w.as_ref());
        self.inner.action.set_visible(true);
    }

    /// Always put the action under the text, also when both fit on one line (Settings' "Bluetooth reageert
    /// niet": `.st-bt-stuck .fl-infobar__action`).
    pub fn set_action_below(&self, below: bool) {
        self.inner.body.set_below(below);
    }

    pub fn set_closable(&self, closable: bool) {
        self.inner.close.set_visible(closable);
    }

    pub fn connect_closed(&self, f: impl Fn() + 'static) {
        self.inner.on_close.borrow_mut().push(Rc::new(f));
    }

    pub fn show(&self) {
        self.root.set_visible(true);
    }

    pub fn set_visible(&self, v: bool) {
        self.root.set_visible(v);
    }
}

/// An in-app toast (components.js Toast.show): bottom centre of the window, title, message, a severity disc or a
/// glyph, an optional action button and a close button; it goes after `timeout_ms` (0: stays), not while the
/// pointer is on it.
pub struct Toast {
    title: String,
    message: String,
    severity: Option<Severity>,
    glyph: String,
    action: Option<(String, Rc<dyn Fn()>)>,
    timeout_ms: u32,
}

/// A shown toast (close it early).
#[derive(Clone)]
pub struct ToastHandle {
    el: glib::WeakRef<gtk::Box>,
}

impl ToastHandle {
    pub fn close(&self) {
        if let Some(el) = self.el.upgrade() {
            remove_toast(&el);
        }
    }
}

fn remove_toast(el: &gtk::Box) {
    if let Some(stack) = el.parent().and_downcast::<gtk::Box>() {
        stack.remove(el);
        stack.set_visible(stack.first_child().is_some());
    }
}

impl Toast {
    pub fn new(title: &str) -> Toast {
        Toast { title: title.into(), message: String::new(), severity: None, glyph: String::new(), action: None, timeout_ms: 5000 }
    }
    pub fn message(mut self, m: &str) -> Toast {
        self.message = m.into();
        self
    }
    pub fn severity(mut self, s: Severity) -> Toast {
        self.severity = Some(s);
        self
    }
    pub fn glyph(mut self, g: &str) -> Toast {
        self.glyph = g.into();
        self
    }
    pub fn action(mut self, label: &str, f: impl Fn() + 'static) -> Toast {
        self.action = Some((label.into(), Rc::new(f)));
        self
    }
    pub fn timeout(mut self, ms: u32) -> Toast {
        self.timeout_ms = ms;
        self
    }

    /// Show it in the window `parent` is in (else the active window).
    pub fn show(self, parent: &impl IsA<gtk::Widget>) -> ToastHandle {
        let Some(window) = AppWindow::of(parent).or_else(AppWindow::active) else {
            return ToastHandle { el: glib::WeakRef::new() };
        };
        let el = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        el.add_css_class("w11-toast");
        el.set_size_request(360, -1);
        if let Some(s) = self.severity {
            let i = icons::severity_icon(s.as_str(), 16);
            i.set_valign(gtk::Align::Start);
            i.set_margin_top(2);
            el.append(&i);
        } else if !self.glyph.is_empty() {
            let g = icons::glyph(&self.glyph, 20);
            g.set_valign(gtk::Align::Start);
            el.append(&g);
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
        body.set_hexpand(true);
        if !self.title.is_empty() {
            body.append(&super::label(&self.title, "w11-toast-title", true));
        }
        if !self.message.is_empty() {
            body.append(&super::label(&self.message, "w11-toast-message", true));
        }
        el.append(&body);
        let close = super::buttons::icon_button("close", "", 28);
        close.add_css_class("w11-toast-close");
        close.set_valign(gtk::Align::Start);
        i18n::tooltip(&close, "Sluiten", "Close");
        el.append(&close);
        let handle = ToastHandle { el: el.downgrade() };
        if let Some((label, f)) = self.action {
            let b = super::buttons::button(&label, super::Variant::Standard);
            b.add_css_class("w11-toast-action");
            b.set_halign(gtk::Align::Start);
            let h = handle.clone();
            b.connect_clicked(move |_| {
                f();
                h.close();
            });
            body.append(&b);
        }
        let h = handle.clone();
        close.connect_clicked(move |_| h.close());
        let stack = window.toast_stack();
        stack.append(&el);
        stack.set_visible(true);
        let timeout = self.timeout_ms;
        if timeout > 0 {
            let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
            let arm = {
                let timer = timer.clone();
                let h = handle.clone();
                move |ms: u32| {
                    let h = h.clone();
                    let t2 = timer.clone();
                    let id = glib::timeout_add_local_once(Duration::from_millis(ms as u64), move || {
                        t2.borrow_mut().take();
                        h.close();
                    });
                    if let Some(old) = timer.borrow_mut().replace(id) {
                        old.remove();
                    }
                }
            };
            arm(timeout);
            let motion = gtk::EventControllerMotion::new();
            let t3 = timer.clone();
            motion.connect_enter(move |_, _, _| {
                if let Some(id) = t3.borrow_mut().take() {
                    id.remove();
                }
            });
            let arm2 = arm.clone();
            motion.connect_leave(move |_| arm2(timeout / 2));
            el.add_controller(motion);
        }
        handle
    }
}
