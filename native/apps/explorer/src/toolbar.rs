//! The address row (back, forward, up, refresh; the breadcrumb address bar with its sub folder menus, the
//! overflow menu, edit mode with suggestions and the recent locations; the search box) and the command bar
//! (Nieuw, the clipboard row, the Prullenbak row, Sorteren, Weergeven, "..." and Details), after
//! 6ebf169f:vendor/ui/js/apps/explorer/toolbar.js.
use crate::explorer::{Explorer, Resolved, WeakExplorer};
use crate::i18n::{f, s};
use crate::menus;
use crate::model::Loc;
use crate::{dnd, ops};
use gtk::prelude::*;
use gtk::{gdk, glib};
use kit::controls::{MenuItem, SearchBox, show_menu_for};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn nav_button(glyph: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("xp-navbtn");
    b.set_child(Some(&kit::icons::glyph_stroke(glyph, 16, 1.3)));
    b.set_widget_name(name);
    b.set_focus_on_click(false);
    b.set_valign(gtk::Align::Center);
    b
}

/// A glyph for a location in a menu (MenuItem takes glyph names).
fn loc_glyph(loc: &Loc) -> &'static str {
    match loc {
        Loc::Home => "home",
        Loc::Gallery => "image",
        Loc::ThisPc => "pc",
        Loc::Trash => "trash",
        Loc::Search { .. } => "search",
        Loc::Folder(_) => "folder",
    }
}

fn with(w: &WeakExplorer, f: impl FnOnce(&Explorer)) {
    if let Some(x) = w.upgrade() {
        f(&x);
    }
}

// ================================================================================================ address

struct AddrInner {
    x: WeakExplorer,
    root: gtk::Box,
    back: gtk::Button,
    fwd: gtk::Button,
    up: gtk::Button,
    refresh: gtk::Button,
    addr: gtk::Box,
    crumbs: gtk::Box,
    input: gtk::Text,
    hist: gtk::Button,
    search: SearchBox,
    sug: gtk::Popover,
    sug_list: gtk::Box,
    sug_items: RefCell<Vec<(String, gtk::Button)>>,
    sug_active: Cell<Option<usize>>,
    sug_seq: Cell<u64>,
    sug_timer: RefCell<Option<glib::SourceId>>,
    editing: Cell<bool>,
    groups: RefCell<Vec<(gtk::Box, String, Loc)>>,
    more: RefCell<Option<gtk::Button>>,
    fixed: RefCell<Vec<gtk::Widget>>,
    narrow: Cell<u8>,
    search_focused: Cell<bool>,
}

/// The address row; a cheap handle.
#[derive(Clone)]
pub struct AddressRow {
    inner: Rc<AddrInner>,
}

impl AddressRow {
    pub fn new(x: WeakExplorer) -> AddressRow {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("xp-addrrow");
        let navbtns = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        navbtns.add_css_class("xp-navbtns");
        let back = nav_button("arrow-left", "explorer-back");
        let fwd = nav_button("arrow-right", "explorer-forward");
        let up = nav_button("arrow-up", "explorer-up");
        let refresh = nav_button("refresh", "explorer-refresh");
        for b in [&back, &fwd, &up, &refresh] {
            navbtns.append(b);
        }
        root.append(&navbtns);

        let addr = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        addr.add_css_class("xp-addr");
        addr.set_hexpand(true);
        addr.set_widget_name("explorer-address");
        addr.set_cursor_from_name(Some("text"));
        let wrap = gtk::Overlay::new();
        wrap.set_hexpand(true);
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        crumbs.add_css_class("xp-crumbs");
        crumbs.set_overflow(gtk::Overflow::Hidden);
        crumbs.set_hexpand(true);
        crumbs.set_cursor_from_name(Some("default"));
        wrap.set_child(Some(&crumbs));
        let probe = gtk::DrawingArea::new();
        probe.set_can_target(false);
        wrap.add_overlay(&probe);
        let input = gtk::Text::new();
        input.add_css_class("xp-addr-input");
        input.set_hexpand(true);
        input.set_visible(false);
        input.set_widget_name("explorer-address-input");
        let hist = gtk::Button::new();
        hist.add_css_class("xp-addr-hist");
        hist.set_child(Some(&kit::icons::glyph("chevron-down", 12)));
        hist.set_focus_on_click(false);
        hist.set_can_focus(false);
        addr.append(&wrap);
        addr.append(&input);
        addr.append(&hist);
        root.append(&addr);

        let search = SearchBox::new(None);
        search.widget().add_css_class("xp-search");
        search.widget().set_widget_name("explorer-search");
        search.text_widget().set_widget_name("explorer-search-input");
        search.set_debounce(120);
        root.append(search.widget());

        let sug = gtk::Popover::new();
        sug.add_css_class("w11-popup");
        sug.add_css_class("xp-sug-popup");
        sug.set_has_arrow(false);
        sug.set_autohide(false);
        sug.set_can_focus(false);
        sug.set_position(gtk::PositionType::Bottom);
        sug.set_halign(gtk::Align::Start);
        let sug_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sug_list.add_css_class("xp-sug");
        sug.set_child(Some(&sug_list));
        sug.set_parent(&addr);

        let inner = Rc::new(AddrInner {
            x,
            root,
            back,
            fwd,
            up,
            refresh,
            addr,
            crumbs,
            input,
            hist,
            search,
            sug,
            sug_list,
            sug_items: RefCell::new(Vec::new()),
            sug_active: Cell::new(None),
            sug_seq: Cell::new(0),
            sug_timer: RefCell::new(None),
            editing: Cell::new(false),
            groups: RefCell::new(Vec::new()),
            more: RefCell::new(None),
            fixed: RefCell::new(Vec::new()),
            narrow: Cell::new(0),
            search_focused: Cell::new(false),
        });
        let me = AddressRow { inner };
        let wk = Rc::downgrade(&me.inner);
        probe.connect_resize(move |_, _, _| {
            if let Some(i) = wk.upgrade() {
                let me = AddressRow { inner: i };
                glib::idle_add_local_once(move || me.fit_crumbs());
            }
        });
        me.wire();
        me.relabel();
        me
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    fn x(&self) -> Option<Explorer> {
        self.inner.x.upgrade()
    }

    /// A weak handle for closures on our own widgets (a strong one would be a cycle).
    fn hold(&self) -> impl Fn() -> Option<AddressRow> + Clone + 'static {
        let w = Rc::downgrade(&self.inner);
        move || w.upgrade().map(|inner| AddressRow { inner })
    }

    fn wire(&self) {
        let i = &self.inner;
        let w = i.x.clone();
        i.back.connect_clicked(move |_| with(&w, |x| x.back()));
        let w = i.x.clone();
        i.fwd.connect_clicked(move |_| with(&w, |x| x.forward()));
        let w = i.x.clone();
        i.up.connect_clicked(move |_| with(&w, |x| x.up()));
        let w = i.x.clone();
        i.refresh.connect_clicked(move |_| with(&w, |x| x.refresh()));
        // right click on back / forward: the history (Windows)
        for (b, dir) in [(&i.back, -1i64), (&i.fwd, 1i64)] {
            let g = gtk::GestureClick::new();
            g.set_button(gdk::BUTTON_SECONDARY);
            let w = i.x.clone();
            let bw = b.downgrade();
            g.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                let (Some(x), Some(b)) = (w.upgrade(), bw.upgrade()) else { return };
                let items: Vec<MenuItem> = x
                    .history_items(dir)
                    .into_iter()
                    .map(|(label, loc, steps)| {
                        let w = x.weak();
                        MenuItem::new(&label, move || with(&w, |x| x.go(steps))).glyph(loc_glyph(&loc))
                    })
                    .collect();
                if !items.is_empty() {
                    show_menu_for(&b, items);
                }
            });
            b.add_controller(g);
        }
        // a click on the address bar (not a crumb) edits it
        let g = gtk::GestureClick::new();
        let hold = self.hold();
        g.connect_released(move |_, _, _, _| {
            if let Some(me) = hold()
                && !me.inner.editing.get()
            {
                me.edit();
            }
        });
        i.addr.add_controller(g);
        // recent locations
        let hold = self.hold();
        i.hist.connect_clicked(move |_| {
            let Some(me) = hold() else { return };
            let Some(x) = me.x() else { return };
            let items: Vec<MenuItem> = x
                .recent_locations()
                .into_iter()
                .map(|loc| {
                    let w = x.weak();
                    let l = loc.clone();
                    MenuItem::new(&x.loc_label(&loc, true), move || with(&w, |x| x.go_to(l.clone()))).glyph(loc_glyph(&loc))
                })
                .collect();
            if !items.is_empty() {
                let p = show_menu_for(&me.inner.addr, items);
                p.set_halign(gtk::Align::End);
            }
        });
        // edit mode keys
        // capture: before the text widget's own Enter handling
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let hold = self.hold();
        keys.connect_key_pressed(move |_, key, _, state| match hold() {
            Some(me) => me.input_key(key, state),
            None => glib::Propagation::Proceed,
        });
        i.input.add_controller(keys);
        let hold = self.hold();
        i.input.connect_changed(move |_| {
            if let Some(me) = hold() {
                me.suggest_soon();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let hold = self.hold();
        focus.connect_leave(move |_| {
            let hold = hold.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if let Some(me) = hold()
                    && !me.inner.input.has_focus()
                {
                    me.end_edit();
                }
            });
        });
        i.input.add_controller(focus);
        // search
        let w = i.x.clone();
        i.search.connect_changed(move |v| with(&w, |x| x.set_filter(v)));
        let w = i.x.clone();
        i.search.connect_submit(move |v| with(&w, |x| x.search_submit(v)));
        let focus = gtk::EventControllerFocus::new();
        let hold = self.hold();
        focus.connect_enter(move |_| {
            if let Some(me) = hold() {
                me.inner.search_focused.set(true);
                me.size_search();
            }
        });
        let hold = self.hold();
        focus.connect_leave(move |_| {
            if let Some(me) = hold() {
                me.inner.search_focused.set(false);
                me.size_search();
            }
        });
        i.search.widget().add_controller(focus);
        self.size_search();
    }

    /// Texts after a language switch.
    pub fn relabel(&self) {
        let i = &self.inner;
        i.refresh.set_tooltip_text(Some(s("refresh")));
        i.hist.set_tooltip_text(Some(s("recentLocations")));
        i.input.update_property(&[gtk::accessible::Property::Label(s("address"))]);
        i.addr.update_property(&[gtk::accessible::Property::Label(s("address"))]);
        self.update();
    }

    /// <860: 200 (240 focused); <560: 140; Forward hidden below 560.
    pub fn set_narrow(&self, level: u8) {
        self.inner.narrow.set(level);
        self.inner.fwd.set_visible(level < 3);
        self.size_search();
    }

    fn size_search(&self) {
        let focused = self.inner.search_focused.get();
        let w = match self.inner.narrow.get() {
            0 => {
                if focused {
                    340
                } else {
                    260
                }
            }
            1 | 2 => {
                if focused {
                    240
                } else {
                    200
                }
            }
            _ => 140,
        };
        self.inner.search.widget().set_size_request(w, -1);
        if focused {
            self.inner.search.widget().add_css_class("is-focused");
        } else {
            self.inner.search.widget().remove_css_class("is-focused");
        }
    }

    /// Buttons, tooltips, the breadcrumb and the search placeholder for the active tab.
    pub fn update(&self) {
        let Some(x) = self.x() else { return };
        let Some(tab) = x.active_tab() else { return };
        let i = &self.inner;
        let (loc, prev, next) = {
            let t = tab.borrow();
            let prev = if t.h_index > 0 { t.history.get(t.h_index - 1).cloned() } else { None };
            (t.loc.clone(), prev, t.history.get(t.h_index + 1).cloned())
        };
        i.back.set_sensitive(prev.is_some());
        i.fwd.set_sensitive(next.is_some());
        let parent = x.parent_loc(&loc);
        i.up.set_sensitive(parent.is_some());
        i.back.set_tooltip_text(Some(&match &prev {
            Some(l) => f("backTo", &[("name", &x.loc_label(l, false))]),
            None => s("back").into(),
        }));
        i.fwd.set_tooltip_text(Some(&match &next {
            Some(l) => f("forwardTo", &[("name", &x.loc_label(l, false))]),
            None => s("forward").into(),
        }));
        i.up.set_tooltip_text(Some(&match &parent {
            Some(l) => f("upTo", &[("name", &x.loc_label(l, false))]),
            None => s("up").into(),
        }));
        if !(i.editing.get() && i.input.has_focus()) {
            self.end_edit();
            self.render_crumbs(&x, &loc);
        }
        let ph = f("searchIn", &[("name", &x.loc_label(&loc, false))]);
        i.search.set_placeholder(&ph);
        i.search.text_widget().update_property(&[gtk::accessible::Property::Label(&ph)]);
        let filter = x.filter();
        if i.search.text() != filter && !i.search.text_widget().has_focus() {
            i.search.set_text(&filter);
        }
    }

    // ------------------------------------------------------------------------------------ breadcrumb

    fn chevron(&self, parent: Option<Loc>) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("xp-crumb-chev");
        let g = kit::icons::glyph("chevron-right", 12);
        b.set_child(Some(&g));
        b.set_can_focus(false);
        b.set_focus_on_click(false);
        let hold = self.hold();
        b.connect_clicked(move |b| {
            let Some(me) = hold() else { return };
            let Some(x) = me.x() else { return };
            b.add_css_class("is-open");
            if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                kit::icons::set_glyph(&img, "chevron-down");
            }
            let bw = b.downgrade();
            subfolder_menu(&x, parent.clone(), move |items| {
                let Some(b) = bw.upgrade() else { return };
                let items = if items.is_empty() { vec![MenuItem::new(s("noSubfolders"), || {}).enabled(false)] } else { items };
                let p = show_menu_for(&b, items);
                let bw = b.downgrade();
                p.connect_closed(move |_| {
                    if let Some(b) = bw.upgrade() {
                        b.remove_css_class("is-open");
                        if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                            kit::icons::set_glyph(&img, "chevron-right");
                        }
                    }
                });
            });
        });
        b
    }

    fn render_crumbs(&self, x: &Explorer, loc: &Loc) {
        let i = &self.inner;
        kit::controls::clear(&i.crumbs);
        i.groups.borrow_mut().clear();
        let parts = x.crumbs_for(loc);
        let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_box.add_css_class("xp-crumbs-icon");
        icon_box.set_valign(gtk::Align::Center);
        icon_box.append(&x.loc_icon(loc, 16));
        i.crumbs.append(&icon_box);
        let more = gtk::Button::new();
        more.add_css_class("xp-crumbs-more");
        let mb = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mb.append(&kit::icons::glyph("chevron-left", 12));
        let second = kit::icons::glyph("chevron-left", 12);
        second.add_css_class("xp-crumbs-more-2");
        mb.append(&second);
        more.set_child(Some(&mb));
        more.set_visible(false);
        more.set_can_focus(false);
        more.set_tooltip_text(Some(s("hiddenParts")));
        i.crumbs.append(&more);
        let root_chev = self.chevron(None);
        i.crumbs.append(&root_chev);
        *i.fixed.borrow_mut() = vec![icon_box.upcast(), root_chev.upcast()];
        let last = parts.len().saturating_sub(1);
        for (n, (label, l)) in parts.into_iter().enumerate() {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("xp-crumbs-seg");
            let seg = gtk::Button::with_label(&label);
            seg.add_css_class("xp-crumb");
            if n == last {
                seg.add_css_class("is-current");
            }
            seg.set_can_focus(false);
            seg.set_focus_on_click(false);
            if let Some(lab) = seg.child().and_downcast::<gtk::Label>() {
                lab.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lab.set_max_width_chars(36);
            }
            seg.set_widget_name(&format!("crumb-{n}"));
            let w = i.x.clone();
            let lc = l.clone();
            seg.connect_clicked(move |_| with(&w, |x| x.go_to(lc.clone())));
            let mid = gtk::GestureClick::new();
            mid.set_button(gdk::BUTTON_MIDDLE);
            let w = i.x.clone();
            let lc = l.clone();
            mid.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                with(&w, |x| {
                    x.open_tab(lc.clone(), false);
                });
            });
            seg.add_controller(mid);
            if let Some(x) = i.x.upgrade() {
                dnd::attach_crumb(&x, seg.upcast_ref(), &l);
            }
            group.append(&seg);
            if n < last {
                group.append(&self.chevron(Some(l.clone())));
            }
            i.crumbs.append(&group);
            i.groups.borrow_mut().push((group, label, l));
        }
        let hold = self.hold();
        more.connect_clicked(move |b| {
            let Some(me) = hold() else { return };
            let Some(x) = me.x() else { return };
            let items: Vec<MenuItem> = me
                .inner
                .groups
                .borrow()
                .iter()
                .filter(|(g, _, _)| !g.is_visible())
                .rev()
                .map(|(_, label, l)| {
                    let w = x.weak();
                    let lc = l.clone();
                    MenuItem::new(label, move || with(&w, |x| x.go_to(lc.clone()))).glyph(loc_glyph(l))
                })
                .collect();
            if !items.is_empty() {
                show_menu_for(b, items);
            }
        });
        *i.more.borrow_mut() = Some(more);
        let me = self.clone();
        glib::idle_add_local_once(move || me.fit_crumbs());
    }

    /// Hide leading segments that do not fit (Windows shows « with a menu).
    fn fit_crumbs(&self) {
        let i = &self.inner;
        let avail = i.crumbs.width();
        if avail <= 0 {
            return;
        }
        let groups = i.groups.borrow();
        for (g, _, _) in groups.iter() {
            g.set_visible(true);
        }
        let nat = |w: &gtk::Widget| w.measure(gtk::Orientation::Horizontal, -1).1;
        let fixed: i32 = i.fixed.borrow().iter().map(nat).sum();
        let more_w = i.more.borrow().as_ref().map(|m| nat(m.upcast_ref())).unwrap_or(24);
        let widths: Vec<i32> = groups.iter().map(|(g, _, _)| nat(g.upcast_ref())).collect();
        let mut total: i32 = fixed + widths.iter().sum::<i32>();
        let mut hidden = 0;
        while total > avail && hidden + 1 < groups.len() {
            if hidden == 0 {
                total += more_w;
            }
            groups[hidden].0.set_visible(false);
            total -= widths[hidden];
            hidden += 1;
        }
        if let Some(m) = i.more.borrow().as_ref() {
            m.set_visible(hidden > 0);
        }
    }

    // ------------------------------------------------------------------------------------- edit mode

    /// Ctrl+L, Alt+D, F4: edit the address.
    pub fn focus_address(&self) {
        self.edit();
    }

    fn edit(&self) {
        let i = &self.inner;
        if i.editing.get() {
            i.input.select_region(0, -1);
            return;
        }
        let Some(x) = self.x() else { return };
        i.editing.set(true);
        i.addr.add_css_class("is-editing");
        if let Some(w) = i.crumbs.parent() {
            w.set_visible(false);
        }
        i.input.set_visible(true);
        i.input.set_text(&x.address_text(&x.loc()));
        i.input.grab_focus();
        i.input.select_region(0, -1);
        self.close_sug();
    }

    /// Back to the breadcrumb.
    pub fn end_edit(&self) {
        let i = &self.inner;
        self.close_sug();
        if !i.editing.replace(false) {
            return;
        }
        i.addr.remove_css_class("is-editing");
        i.input.set_visible(false);
        if let Some(w) = i.crumbs.parent() {
            w.set_visible(true);
        }
    }


    fn input_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let i = &self.inner;
        let n = i.sug_items.borrow().len();
        match key {
            gdk::Key::Escape => {
                self.end_edit();
                if let Some(x) = self.x() {
                    x.focus_view();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(a) = i.sug_active.get()
                    && let Some((v, _)) = i.sug_items.borrow().get(a)
                {
                    i.input.set_text(v);
                }
                self.go();
                glib::Propagation::Stop
            }
            gdk::Key::Down if n > 0 => {
                let a = i.sug_active.get().map(|a| (a + 1).min(n - 1)).unwrap_or(0);
                self.mark_sug(Some(a));
                glib::Propagation::Stop
            }
            gdk::Key::Up if n > 0 => {
                let a = match i.sug_active.get() {
                    Some(0) | None => None,
                    Some(a) => Some(a - 1),
                };
                self.mark_sug(a);
                glib::Propagation::Stop
            }
            gdk::Key::Tab if n > 0 && !state.contains(gdk::ModifierType::SHIFT_MASK) => {
                let v = i.sug_items.borrow().get(i.sug_active.get().unwrap_or(0)).map(|(v, _)| v.clone());
                if let Some(v) = v {
                    let v = if v.ends_with('/') { v } else { format!("{v}/") };
                    i.input.set_text(&v);
                    i.input.set_position(-1);
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn go(&self) {
        let Some(x) = self.x() else { return };
        let text = self.inner.input.text().to_string();
        let typed = text.clone();
        let me = self.clone();
        x.resolve_address(&text, move |x, r| match r {
            Resolved::Go(loc) => {
                me.end_edit();
                x.go_to(loc);
                x.focus_view();
            }
            Resolved::Handled => {
                me.end_edit();
                x.focus_view();
            }
            Resolved::NotFound => {
                kit::controls::alert(&x.anchor(), s("appName"), &f("cantFind", &[("path", &typed)]), || {});
            }
        });
    }

    fn suggest_soon(&self) {
        let i = &self.inner;
        if !i.editing.get() {
            return;
        }
        if let Some(t) = i.sug_timer.borrow_mut().take() {
            t.remove();
        }
        let me = self.clone();
        let src = glib::timeout_add_local_once(Duration::from_millis(120), move || {
            me.inner.sug_timer.borrow_mut().take();
            let Some(x) = me.x() else { return };
            let seq = me.inner.sug_seq.get() + 1;
            me.inner.sug_seq.set(seq);
            let text = me.inner.input.text().to_string();
            let me2 = me.clone();
            x.address_suggestions(&text, move |x, list| {
                if me2.inner.sug_seq.get() != seq || !me2.inner.editing.get() {
                    return;
                }
                me2.show_sug(x, list);
            });
        });
        *i.sug_timer.borrow_mut() = Some(src);
    }

    fn show_sug(&self, x: &Explorer, list: Vec<(String, String, Loc)>) {
        let i = &self.inner;
        kit::controls::clear(&i.sug_list);
        i.sug_items.borrow_mut().clear();
        i.sug_active.set(None);
        if list.is_empty() {
            i.sug.popdown();
            return;
        }
        for (label, value, loc) in list {
            let b = gtk::Button::new();
            b.add_css_class("w11-list-item");
            b.add_css_class("xp-sug-item");
            b.set_can_focus(false);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("w11-item-body");
            row.append(&x.loc_icon(&loc, 16));
            row.append(&kit::controls::label(&label, "xp-sug-text", false));
            b.set_child(Some(&row));
            let hold = self.hold();
            let v = value.clone();
            b.connect_clicked(move |_| {
                if let Some(me) = hold() {
                    me.inner.input.set_text(&v);
                    me.close_sug();
                    me.go();
                }
            });
            i.sug_list.append(&b);
            i.sug_items.borrow_mut().push((value, b));
        }
        let w = i.addr.width();
        if w > 0 {
            i.sug_list.set_size_request(w - 8, -1);
        }
        i.sug.popup();
    }

    fn mark_sug(&self, a: Option<usize>) {
        self.inner.sug_active.set(a);
        for (n, (_, b)) in self.inner.sug_items.borrow().iter().enumerate() {
            if Some(n) == a {
                b.add_css_class("w11-active");
            } else {
                b.remove_css_class("w11-active");
            }
        }
    }

    fn close_sug(&self) {
        self.inner.sug.popdown();
        self.inner.sug_items.borrow_mut().clear();
        self.inner.sug_active.set(None);
    }

    // ---------------------------------------------------------------------------------------- search

    /// Ctrl+F, Ctrl+E, F3.
    pub fn focus_search(&self) {
        self.inner.search.focus();
        self.inner.search.text_widget().select_region(0, -1);
    }


    pub fn clear_search(&self) {
        self.inner.search.set_text("");
    }
}

/// The sub folder menu of a crumb's chevron (None: the root chevron with the places).
fn subfolder_menu(x: &Explorer, loc: Option<Loc>, done: impl FnOnce(Vec<MenuItem>) + 'static) {
    let nav = |x: &Explorer, l: Loc| {
        let w = x.weak();
        move || with(&w, |x| x.go_to(l.clone()))
    };
    match loc {
        None => {
            let mut out = vec![
                MenuItem::new(s("home"), nav(x, Loc::Home)).glyph("home"),
                MenuItem::new(s("gallery"), nav(x, Loc::Gallery)).glyph("image"),
                MenuItem::separator(),
            ];
            for (p, label, _) in x.pinned_list() {
                out.push(MenuItem::new(&label, nav(x, Loc::Folder(p))).glyph("folder"));
            }
            out.push(MenuItem::separator());
            out.push(MenuItem::new(s("thisPC"), nav(x, Loc::ThisPc)).glyph("pc"));
            if x.place("trash").is_some() {
                out.push(MenuItem::new(s("trash"), nav(x, Loc::Trash)).glyph("trash"));
            }
            done(out);
        }
        Some(Loc::ThisPc) => {
            x.load_volumes(move |x| {
                let mut out: Vec<MenuItem> = x.default_folders().into_iter().map(|(p, l, _)| MenuItem::new(&l, nav(x, Loc::Folder(p))).glyph("folder")).collect();
                out.push(MenuItem::separator());
                let vols = x.state().volumes.clone();
                for v in vols {
                    let m = crate::util::st(&v, "mount").to_owned();
                    out.push(MenuItem::new(&crate::util::volume_label(&v), nav(x, Loc::Folder(m))).glyph("drive"));
                }
                done(out);
            });
        }
        Some(Loc::Folder(p)) => {
            x.list_dirs(&p, move |x, r| {
                let list = r.unwrap_or_default();
                done(list.into_iter().take(60).map(|d| MenuItem::new(&d.name, nav(x, Loc::Folder(d.path))).glyph("folder")).collect());
            });
        }
        Some(_) => done(Vec::new()),
    }
}

// ============================================================================================ command bar

struct CmdInner {
    x: WeakExplorer,
    root: gtk::Box,
    group_new: gtk::Box,
    group_clip: gtk::Box,
    group_trash: gtk::Box,
    new: gtk::Button,
    cut: gtk::Button,
    copy: gtk::Button,
    paste: gtk::Button,
    rename: gtk::Button,
    share: gtk::Button,
    delete: gtk::Button,
    sort: gtk::Button,
    view: gtk::Button,
    more: gtk::Button,
    empty_trash: gtk::Button,
    restore: gtk::Button,
    details: gtk::Button,
    texts: RefCell<Vec<(gtk::Label, &'static str)>>,
}

/// The command bar; a cheap handle.
#[derive(Clone)]
pub struct CommandBar {
    inner: Rc<CmdInner>,
}

fn sep() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("xp-cmdsep");
    b.set_size_request(1, 24);
    b.set_valign(gtk::Align::Center);
    b
}

/// The accent "Nieuw" glyph: a filled circle with a plus.
fn new_glyph() -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(16);
    a.set_content_height(16);
    a.add_css_class("xp-newglyph");
    a.set_draw_func(|area, cr, w, h| {
        let s = w.min(h) as f64 / 16.0;
        cr.scale(s, s);
        let fill = if area.is_sensitive() { kit::theme::token("accent") } else { kit::theme::token("text-disabled") };
        let fg = kit::theme::token("accent-text");
        cr.set_source_rgba(fill.red() as f64, fill.green() as f64, fill.blue() as f64, fill.alpha() as f64);
        cr.arc(8.0, 8.0, 7.25, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, fg.alpha() as f64);
        cr.set_line_width(1.4);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.move_to(8.0, 4.75);
        cr.line_to(8.0, 11.25);
        cr.move_to(4.75, 8.0);
        cr.line_to(11.25, 8.0);
        let _ = cr.stroke();
    });
    kit::theme::track(&a);
    a
}

impl CommandBar {
    pub fn new(x: WeakExplorer) -> CommandBar {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("xp-cmdbar");
        root.set_widget_name("explorer-commands");
        let texts: RefCell<Vec<(gtk::Label, &'static str)>> = RefCell::new(Vec::new());
        let make = |glyph: Option<&str>, custom: Option<gtk::Widget>, text: Option<&'static str>, chevron: bool, name: &str| -> gtk::Button {
            let b = gtk::Button::new();
            b.add_css_class("xp-cmd");
            b.set_widget_name(name);
            b.set_focus_on_click(false);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            icon.add_css_class("xp-cmd-icon");
            icon.set_valign(gtk::Align::Center);
            if let Some(c) = custom {
                icon.append(&c);
            } else if let Some(g) = glyph {
                icon.append(&kit::icons::glyph_stroke(g, 16, 1.3));
            }
            row.append(&icon);
            if let Some(t) = text {
                b.add_css_class("has-text");
                let l = gtk::Label::new(Some(s(t)));
                l.add_css_class("xp-cmd-text");
                row.append(&l);
                texts.borrow_mut().push((l, t));
            }
            if chevron {
                let c = kit::icons::glyph("chevron-down", 12);
                c.add_css_class("xp-cmd-chev");
                row.append(&c);
            }
            b.set_child(Some(&row));
            b
        };
        let new = make(None, Some(new_glyph().upcast()), Some("new"), true, "explorer-cmd-new");
        new.add_css_class("xp-cmd-new");
        let cut = make(Some("cut"), None, None, false, "explorer-cmd-cut");
        let copy = make(Some("copy"), None, None, false, "explorer-cmd-copy");
        let paste = make(Some("paste"), None, None, false, "explorer-cmd-paste");
        let rename = make(Some("rename"), None, None, false, "explorer-cmd-rename");
        let share = make(Some("share"), None, None, false, "explorer-cmd-share");
        let delete = make(Some("delete"), None, None, false, "explorer-cmd-delete");
        let sort = make(Some("sort"), None, Some("sort"), true, "explorer-cmd-sort");
        let view = make(Some("view"), None, Some("view"), true, "explorer-cmd-view");
        let more = make(Some("more"), None, None, false, "explorer-cmd-more");
        let empty_trash = make(Some("trash"), None, Some("emptyTrash"), false, "explorer-cmd-empty-trash");
        let restore = make(Some("undo"), None, Some("restoreAll"), false, "explorer-cmd-restore");
        let details = make(Some("properties"), None, Some("details"), false, "explorer-cmd-details");
        details.add_css_class("xp-cmd-details");
        let group = |ws: &[&gtk::Button], with_sep: bool| {
            let g = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            g.add_css_class("xp-cmdgroup");
            for w in ws {
                g.append(*w);
            }
            if with_sep {
                g.append(&sep());
            }
            g
        };
        let group_new = group(&[&new], true);
        let group_clip = group(&[&cut, &copy, &paste, &rename, &share, &delete], true);
        let group_trash = group(&[&empty_trash, &restore], true);
        let group_view = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        group_view.add_css_class("xp-cmdgroup");
        group_view.append(&sort);
        group_view.append(&view);
        group_view.append(&sep());
        group_view.append(&more);
        root.append(&group_new);
        root.append(&group_clip);
        root.append(&group_trash);
        root.append(&group_view);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        root.append(&spacer);
        root.append(&details);
        let inner = Rc::new(CmdInner {
            x,
            root,
            group_new,
            group_clip,
            group_trash,
            new,
            cut,
            copy,
            paste,
            rename,
            share,
            delete,
            sort,
            view,
            more,
            empty_trash,
            restore,
            details,
            texts,
        });
        let me = CommandBar { inner };
        me.wire();
        me.relabel();
        me
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    fn wire(&self) {
        let i = &self.inner;
        let menu = |b: &gtk::Button, build: fn(&Explorer) -> Vec<MenuItem>| {
            let w = i.x.clone();
            b.connect_clicked(move |b| {
                let Some(x) = w.upgrade() else { return };
                b.add_css_class("is-open");
                let p = show_menu_for(b, build(&x));
                let bw = b.downgrade();
                p.connect_closed(move |_| {
                    if let Some(b) = bw.upgrade() {
                        b.remove_css_class("is-open");
                    }
                });
            });
        };
        menu(&i.new, menus::new_menu);
        menu(&i.sort, menus::sort_menu);
        menu(&i.view, menus::view_menu);
        menu(&i.more, menus::more_menu);
        let click = |b: &gtk::Button, f: fn(&Explorer)| {
            let w = i.x.clone();
            b.connect_clicked(move |_| with(&w, f));
        };
        click(&i.cut, ops::cut);
        click(&i.copy, ops::copy);
        click(&i.paste, |x| ops::paste(x, None));
        click(&i.rename, |x| x.rename_selected());
        click(&i.share, |x| crate::dialogs::show_share(x, x.selected_items()));
        click(&i.delete, ops::trash);
        click(&i.empty_trash, ops::empty_trash);
        click(&i.restore, |x| ops::restore(x, None));
        click(&i.details, |x| x.toggle_cfg("detailsPane"));
        // Left / Right move through the buttons
        let keys = gtk::EventControllerKey::new();
        let root = i.root.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            let d: i32 = match key {
                gdk::Key::Right => 1,
                gdk::Key::Left => -1,
                _ => return glib::Propagation::Proceed,
            };
            let Some(root) = root.upgrade() else { return glib::Propagation::Proceed };
            let mut btns = Vec::new();
            fn walk(w: &gtk::Widget, out: &mut Vec<gtk::Button>) {
                if let Some(b) = w.downcast_ref::<gtk::Button>() {
                    if b.is_sensitive() && b.is_mapped() {
                        out.push(b.clone());
                    }
                    return;
                }
                let mut c = w.first_child();
                while let Some(ch) = c {
                    walk(&ch, out);
                    c = ch.next_sibling();
                }
            }
            walk(root.upcast_ref(), &mut btns);
            let Some(cur) = btns.iter().position(|b| b.has_focus()) else { return glib::Propagation::Proceed };
            let n = btns.len() as i32;
            btns[((cur as i32 + d + n) % n) as usize].grab_focus();
            glib::Propagation::Stop
        });
        i.root.add_controller(keys);
    }

    /// Texts and tooltips after a language switch.
    pub fn relabel(&self) {
        let i = &self.inner;
        for (l, k) in i.texts.borrow().iter() {
            l.set_label(s(k));
        }
        i.new.set_tooltip_text(Some(s("newHint")));
        i.cut.set_tooltip_text(Some(s("cut")));
        i.copy.set_tooltip_text(Some(s("copy")));
        i.paste.set_tooltip_text(Some(s("paste")));
        i.rename.set_tooltip_text(Some(s("rename")));
        i.share.set_tooltip_text(Some(s("share")));
        i.delete.set_tooltip_text(Some(s("delete")));
        i.more.set_tooltip_text(Some(s("more")));
        i.details.set_tooltip_text(Some(s("detailsHint")));
        i.root.update_property(&[gtk::accessible::Property::Label(s("commandBar"))]);
        self.update();
    }

    /// Below 700 px only the icons.
    pub fn set_narrow(&self, narrow: bool) {
        for (l, _) in self.inner.texts.borrow().iter() {
            l.set_visible(!narrow);
        }
    }

    /// What is possible here (main.js CommandBar.update).
    pub fn update(&self) {
        let Some(x) = self.inner.x.upgrade() else { return };
        let Some(tab) = x.active_tab() else { return };
        let i = &self.inner;
        let loc = tab.borrow().loc.clone();
        let has_items = tab.borrow().items.as_ref().is_some_and(|v| !v.is_empty());
        let n = x.selected_items().len();
        let kind = loc.kind();
        let trash = loc == Loc::Trash;
        let folder = loc.is_folder();
        let fileish = loc.is_fileish();
        i.group_new.set_visible(!trash);
        i.group_clip.set_visible(!trash);
        i.group_trash.set_visible(trash);
        i.new.set_sensitive(folder);
        i.cut.set_sensitive(fileish && n > 0);
        i.copy.set_sensitive((fileish || kind == "home" || kind == "gallery") && n > 0);
        i.paste.set_sensitive(folder && !x.state().clipboard_paths.is_empty());
        i.rename.set_sensitive(fileish && n == 1);
        i.share.set_sensitive(n > 0 && kind != "thispc");
        i.delete.set_sensitive((fileish && n > 0) || (kind == "gallery" && n > 0));
        i.sort.set_sensitive(fileish || trash);
        i.view.set_sensitive(fileish || trash);
        i.empty_trash.set_sensitive(has_items);
        let restore_label = if n > 0 { s("restoreSel") } else { s("restoreAll") };
        for (l, k) in i.texts.borrow().iter() {
            if *k == "restoreAll" || *k == "restoreSel" {
                l.set_label(restore_label);
            }
        }
        i.restore.set_tooltip_text(Some(restore_label));
        i.restore.set_sensitive(has_items);
        if x.cfg_bool("detailsPane", false) {
            i.details.add_css_class("is-on");
        } else {
            i.details.remove_css_class("is-on");
        }
        // the New glyph greys out with its button
        if let Some(row) = i.new.child()
            && let Some(icon) = row.first_child()
            && let Some(g) = icon.first_child()
        {
            g.queue_draw();
        }
    }
}
