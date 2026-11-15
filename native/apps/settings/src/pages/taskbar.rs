//! Instellingen › Persoonlijke instellingen › Taakbalk (6ebf169f:vendor/ui/js/apps/settings/pages/taskbar.js): the
//! items (search, task view, widgets), the notification area ("Andere systeemvakpictogrammen": which tray icons sit
//! on the taskbar and which stay behind the ^ overflow) and the behaviour (alignment, auto-hide, seconds), with a
//! live mini taskbar on the wallpaper.
//!
//! Deep link: {page: "taskbar", section: "tray"} opens the tray expander and scrolls to it.
use super::background::Thumb;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::widgets::Clamp;
use kit::controls::{self as c, InfoBar, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

pub const PAGES: &[(&str, Builder)] = &[("taskbar", page)];
pub const CSS: &str = r#"
.st-tbprev {
  border-radius: var(--radius-card);
  box-shadow: 0 0 0 1px var(--stroke-card), 0 8px 16px rgba(0, 0, 0, 0.14);
}
.st-tbprev-base { border-radius: var(--radius-card); background-color: transparent; }
.st-tbprev-bar { padding: 0 8px; color: var(--text-1); }
.st-tbprev-btn { min-width: 36px; min-height: 36px; border-radius: var(--radius-control); }
.st-tbprev-btn.st-active { background-color: var(--fill-subtle-2); }
.st-tbprev-search {
  min-height: 30px; margin: 0 4px; padding: 0 12px; border-radius: 15px;
  background-color: var(--fill-control); box-shadow: inset 0 0 0 1px var(--stroke-control);
  color: var(--text-2); font-size: var(--fs-caption);
}
.st-tbprev-search.st-pill { background-color: transparent; box-shadow: none; color: var(--text-1); padding: 0 10px; }
.st-tbprev-widget { padding: 0 8px; font-size: var(--fs-caption); }
.st-tbprev-tray { padding-right: 4px; }
.st-tbprev-clock { padding-left: 4px; font-size: 11px; }
.st-tray-row .w11-sc-icon { margin: 0 12px 0 0; }
"#;

/// params.section values that open "Andere systeemvakpictogrammen".
const TRAY_SECTIONS: &[&str] = &["tray", "systray", "notificationarea"];

fn str_list(path: &str) -> Vec<String> {
    kit::ctx().setting(path).as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn hidden_menu_on() -> bool {
    kit::ctx().setting("taskbar.tray.hiddenMenu") != false
}

/// An item's key (`key || id`), "" when it has none.
fn item_key(it: &Value) -> String {
    for k in ["key", "id"] {
        match &it[k] {
            Value::String(s) if !s.is_empty() => return s.clone(),
            Value::Number(n) => return n.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The notification area as tray.list tells it (the host's role, watcher or host, does not change the page).
struct Tray {
    supported: Cell<bool>,
    items: RefCell<Vec<Value>>,
}

/// tray.list -> (supported, items); a missing method or a failure counts as "not available".
async fn load_tray(p: &Page) -> (bool, Vec<Value>) {
    match p.get("tray.list", json!({})).await {
        Some(v) if v.is_object() => (v["supported"] != false, v["items"].as_array().cloned().unwrap_or_default()),
        _ => (false, Vec::new()),
    }
}

/// One row of the tray section.
struct TrayApp {
    key: String,
    name: String,
    icon: Option<String>,
}

/// Rows: live items and taskbar.tray.known (apps not running now), one per key, sorted by name.
fn tray_apps(items: &[Value]) -> Vec<TrayApp> {
    let label = |key: &str| key.strip_prefix("sni:").or_else(|| key.strip_prefix("exe:")).unwrap_or(key).to_owned();
    let mut apps: Vec<TrayApp> = Vec::new();
    for it in items {
        let key = item_key(it);
        if key.is_empty() || apps.iter().any(|a| a.key == key) {
            continue;
        }
        let name = non_empty(&it["appName"]).or_else(|| non_empty(&it["title"])).unwrap_or_else(|| label(&key));
        let icon = non_empty(&it["appIcon"]).or_else(|| non_empty(&it["icon"]));
        apps.push(TrayApp { key, name, icon });
    }
    if let Some(known) = kit::ctx().setting("taskbar.tray.known").as_object() {
        for (key, k) in known {
            if apps.iter().any(|a| &a.key == key) {
                continue;
            }
            apps.push(TrayApp { key: key.clone(), name: non_empty(&k["name"]).unwrap_or_else(|| label(key)), icon: non_empty(&k["icon"]) });
        }
    }
    apps.sort_by(|a, b| kit::format::compare_natural(&a.name.to_lowercase(), &b.name.to_lowercase()).then_with(|| kit::format::compare_natural(&a.key, &b.key)));
    apps
}

/// What the mini taskbar shows of the notification area: the ^ when icons are hidden, then at most 3 icons in
/// the user's order (taskbar.js trayPreview).
fn tray_preview(items: &[Value]) -> (bool, Vec<Option<String>>) {
    let live: Vec<&Value> = items.iter().filter(|it| !item_key(it).is_empty() && it["status"] != "passive").collect();
    let menu = hidden_menu_on();
    let shown = str_list("taskbar.tray.shown");
    let order = str_list("taskbar.tray.order");
    let rank = |it: &Value| {
        let k = it["key"].as_str().unwrap_or("");
        order.iter().position(|o| o == k).unwrap_or(order.len())
    };
    let mut on_bar: Vec<&Value> = live.iter().copied().filter(|it| !menu || shown.iter().any(|s| Some(s.as_str()) == it["key"].as_str())).collect();
    on_bar.sort_by_key(|it| rank(it));
    let icons = on_bar
        .iter()
        .take(3)
        .map(|it| {
            let attention = if it["status"] == "attention" { non_empty(&it["attentionIcon"]) } else { None };
            attention.or_else(|| non_empty(&it["icon"])).or_else(|| non_empty(&it["appIcon"]))
        })
        .collect();
    (menu && on_bar.len() < live.len(), icons)
}

// ------------------------------------------------------------------------------------- the tray section

struct TrayRow {
    card: SettingsCard,
    sw: ToggleSwitch,
    name: String,
    icon: Option<String>,
}

/// "Andere systeemvakpictogrammen" (Win11 24H2): the "Menu met verborgen pictogrammen" toggle, then one row
/// per app (On: on the taskbar, Off: in the ^ overflow).
struct TraySection {
    p: Page,
    tray: Rc<Tray>,
    exp: SettingsExpander,
    hidden_sw: ToggleSwitch,
    hidden_card: SettingsCard,
    na_bar: InfoBar,
    na_note: gtk::Box,
    all_note: gtk::Label,
    empty_note: gtk::Label,
    rows: RefCell<HashMap<String, TrayRow>>,
    nodes: RefCell<Vec<gtk::Widget>>,
}

impl TraySection {
    fn new(p: &Page, tray: Rc<Tray>, expanded: bool) -> Rc<TraySection> {
        let hidden_sw = ToggleSwitch::new(hidden_menu_on());
        hidden_sw.switch().set_widget_name("taskbar-tray-hidden-menu");
        let hidden_card = SettingsCard::new(
            "",
            t("Menu met verborgen pictogrammen", "Hidden icon menu"),
            t("Pictogrammen die niet op de taakbalk staan, vind je achter de pijl ^", "Icons that aren’t on the taskbar are kept behind the ^ arrow"),
        );
        hidden_card.set_control(hidden_sw.widget());
        let na_bar = common::notice(t(
            "Het systeemvak is in deze sessie niet beschikbaar. Je ziet hier de apps die eerder een pictogram hadden, maar je kunt ze nu niet wijzigen.",
            "The system tray isn’t available in this session. You can see the apps that had an icon before, but you can’t change them right now.",
        ));
        let na_note = gtk::Box::new(gtk::Orientation::Vertical, 0);
        na_note.add_css_class("st-expander-note");
        na_note.append(na_bar.widget());
        let all_note = c::label(
            t(
                "Het menu met verborgen pictogrammen staat uit, dus alle actieve pictogrammen staan op de taakbalk.",
                "The hidden icon menu is off, so all active icons are on the taskbar.",
            ),
            "st-sub-note",
            true,
        );
        let empty_note = c::label(
            t(
                "Er zijn nog geen apps met een pictogram in het systeemvak. Apps zoals Discord, Steam of Nextcloud verschijnen hier zodra ze er een tonen.",
                "No apps have a system tray icon yet. Apps such as Discord, Steam or Nextcloud appear here once they show one.",
            ),
            "st-sub-note",
            true,
        );
        let exp = SettingsExpander::new("apps", t("Andere systeemvakpictogrammen", "Other system tray icons"), t("Extra systeemvakpictogrammen weergeven of verbergen", "Show or hide additional system tray icons"));
        exp.header_button().set_widget_name("taskbar-tray");
        exp.set_expanded(expanded);
        let me = Rc::new(TraySection {
            p: p.clone(),
            tray,
            exp,
            hidden_sw: hidden_sw.clone(),
            hidden_card,
            na_bar,
            na_note,
            all_note,
            empty_note,
            rows: RefCell::new(HashMap::new()),
            nodes: RefCell::new(Vec::new()),
        });
        let (pp, sw) = (p.clone(), hidden_sw.clone());
        hidden_sw.connect_toggled(move |on| {
            let sw = sw.clone();
            common::set_setting(&pp, "taskbar.tray.hiddenMenu", json!(on), move |ok| {
                if !ok {
                    sw.set_active(hidden_menu_on());
                }
            });
        });
        me.paint();
        me
    }

    fn set_shown(self: &Rc<Self>, key: &str, on: bool) {
        let mut next = str_list("taskbar.tray.shown");
        if on {
            if !next.iter().any(|k| k == key) {
                next.push(key.to_owned());
            }
        } else {
            next.retain(|k| k != key);
        }
        let (me, key) = (Rc::downgrade(self), key.to_owned());
        common::set_setting(&self.p, "taskbar.tray.shown", json!(next), move |ok| {
            let Some(me) = me.upgrade() else { return };
            if !ok {
                me.paint();
                return;
            }
            let order = str_list("taskbar.tray.order");
            if on && !order.contains(&key) {
                // keys missing from `order` sort after it on the taskbar (in registration order): write those out
                // first, or the switched-on icon would jump to the front instead of landing at the end
                let mut all = order.clone();
                for k in me.tray.items.borrow().iter().map(item_key) {
                    if !k.is_empty() && k != key && !all.contains(&k) {
                        all.push(k);
                    }
                }
                all.push(key);
                let weak = Rc::downgrade(&me);
                common::set_setting(&me.p, "taskbar.tray.order", json!(all), move |ok| {
                    if !ok && let Some(me) = weak.upgrade() {
                        me.paint();
                    }
                });
            }
        });
    }

    fn row_for(self: &Rc<Self>, app: &TrayApp) -> (SettingsCard, ToggleSwitch) {
        let mut rows = self.rows.borrow_mut();
        if let Some(r) = rows.get_mut(&app.key) {
            if r.name != app.name {
                r.card.set_title(&app.name);
                r.name = app.name.clone();
            }
            if r.icon != app.icon {
                r.card.set_icon_widget(&tray_icon(app.icon.as_deref(), 20));
                r.icon = app.icon.clone();
            }
            return (r.card.clone(), r.sw.clone());
        }
        let sw = ToggleSwitch::new(false);
        sw.switch().set_widget_name(&format!("taskbar-tray-{}", app.key));
        let weak: Weak<TraySection> = Rc::downgrade(self);
        let key = app.key.clone();
        sw.connect_toggled(move |on| {
            if let Some(me) = weak.upgrade() {
                me.set_shown(&key, on);
            }
        });
        let card = SettingsCard::new("", &app.name, "");
        card.set_icon_widget(&tray_icon(app.icon.as_deref(), 20));
        card.widget().add_css_class("st-tray-row");
        card.set_control(sw.widget());
        rows.insert(app.key.clone(), TrayRow { card: card.clone(), sw: sw.clone(), name: app.name.clone(), icon: app.icon.clone() });
        (card, sw)
    }

    fn paint(self: &Rc<Self>) {
        let menu = hidden_menu_on();
        let read_only = !self.tray.supported.get();
        let shown = str_list("taskbar.tray.shown");
        self.hidden_sw.set_active(menu);
        self.hidden_sw.set_sensitive(!read_only);
        let items = self.tray.items.borrow().clone();
        let apps = tray_apps(&items);
        // running apps whose every icon is Passive are shown nowhere until they turn active (the switch keeps the
        // choice for then)
        let live: Vec<&Value> = items.iter().filter(|x| !item_key(x).is_empty()).collect();
        let passive: HashSet<String> = live
            .iter()
            .map(|x| item_key(x))
            .filter(|k| live.iter().all(|x| item_key(x) != *k || x["status"] == "passive"))
            .collect();
        let mut nodes: Vec<gtk::Widget> = Vec::new();
        if read_only {
            nodes.push(self.na_note.clone().upcast());
        }
        nodes.push(self.hidden_card.widget().clone());
        if !menu && !apps.is_empty() {
            nodes.push(self.all_note.clone().upcast());
        }
        for app in &apps {
            let (card, sw) = self.row_for(app);
            let is_passive = passive.contains(&app.key);
            // hidden icon menu off: every active icon is on the taskbar, the per-app choice does not apply
            sw.set_active(if menu { shown.contains(&app.key) } else { !is_passive });
            card.set_description(if is_passive { t("Niet zichtbaar zolang de app niet actief is", "Not shown while the app is inactive") } else { "" });
            card.set_sensitive(menu);
            sw.set_sensitive(!read_only && menu);
            nodes.push(card.widget().clone());
        }
        self.na_bar.set_message(if apps.is_empty() {
            t("Het systeemvak is in deze sessie niet beschikbaar.", "The system tray isn’t available in this session.")
        } else {
            t(
                "Het systeemvak is in deze sessie niet beschikbaar. Je ziet hier de apps die eerder een pictogram hadden, maar je kunt ze nu niet wijzigen.",
                "The system tray isn’t available in this session. You can see the apps that had an icon before, but you can’t change them right now.",
            )
        });
        if apps.is_empty() && !read_only {
            nodes.push(self.empty_note.clone().upcast());
        }
        let keys: HashSet<&str> = apps.iter().map(|a| a.key.as_str()).collect();
        self.rows.borrow_mut().retain(|k, _| keys.contains(k.as_str()));
        // move as little as possible: a focused switch keeps its focus when only its state changed
        if *self.nodes.borrow() != nodes {
            self.exp.clear();
            for n in &nodes {
                self.exp.add(n);
            }
            *self.nodes.borrow_mut() = nodes;
        }
    }
}

/// An app's tray icon, or the "app" glyph.
fn tray_icon(name: Option<&str>, px: i32) -> gtk::Widget {
    match name.filter(|n| !n.is_empty()) {
        Some(n) => kit::icons::app_icon(n, px).upcast(),
        None => kit::icons::glyph("app", px - 2).upcast(),
    }
}

// ------------------------------------------------------------------------------------ the mini taskbar

const BAR_H: i32 = 44;

/// color-mix(in srgb, a <wa>, b): premultiplied, as CSS mixes.
fn mix(a: &gdk::RGBA, b: &gdk::RGBA, wa: f32) -> gdk::RGBA {
    let wb = 1.0 - wa;
    let alpha = a.alpha() * wa + b.alpha() * wb;
    if alpha <= 0.0 {
        return gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
    }
    let ch = |x: f32, y: f32| (x * a.alpha() * wa + y * b.alpha() * wb) / alpha;
    gdk::RGBA::new(ch(a.red(), b.red()), ch(a.green(), b.green()), ch(a.blue(), b.blue()), alpha)
}

struct BarLook {
    tint: gdk::RGBA,
    stroke: gdk::RGBA,
}

struct Preview {
    root: Clamp,
    overlay: gtk::Overlay,
    base: Thumb,
    bar: gtk::CenterBox,
    offset: Rc<Cell<i32>>,
    autohide: Cell<bool>,
    hover: Cell<bool>,
    left_aligned: Cell<bool>,
    look: Rc<RefCell<BarLook>>,
    clock: RefCell<Option<(gtk::Label, Option<gtk::Label>)>>,
    extras: RefCell<Vec<gtk::Widget>>,
    tray: Rc<Tray>,
}

/// A 36 px taskbar button (`.st-tbprev__btn`) around a child.
fn tb_btn(child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("st-tbprev-btn");
    b.set_size_request(36, 36);
    b.set_valign(gtk::Align::Center);
    b.set_hexpand(false);
    let c = child.as_ref();
    c.set_hexpand(true);
    c.set_halign(gtk::Align::Center);
    c.set_valign(gtk::Align::Center);
    b.append(c);
    b
}

/// The running-app mark under a taskbar button: 6 x 3 grey, or 14 x 3 in the accent for the active app.
fn app_mark(active: bool, accent: gdk::RGBA) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(if active { 14 } else { 6 });
    area.set_content_height(3);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::End);
    area.set_margin_bottom(2);
    area.set_can_target(false);
    area.set_draw_func(move |_, cr, w, h| {
        let c = if active { accent } else { kit::theme::token("text-3") };
        let (w, h) = (w as f64, h as f64);
        let r = (h / 2.0).min(2.0);
        cr.new_sub_path();
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        let _ = cr.fill();
    });
    area
}

impl Preview {
    fn new(p: &Page, tray: Rc<Tray>) -> Rc<Preview> {
        let base = Thumb::new(0.0, 132, 640, 8.0);
        base.add_css_class("st-tbprev-base");
        base.set_pos_y(0.7);
        let offset = Rc::new(Cell::new(0));
        let look = Rc::new(RefCell::new(BarLook { tint: gdk::RGBA::new(0.11, 0.11, 0.11, 0.86), stroke: gdk::RGBA::new(1.0, 1.0, 1.0, 0.075) }));
        {
            // the bar's material under its buttons: the tint (no blur here) with a 1 px top stroke
            let (offset, look) = (offset.clone(), look.clone());
            base.set_painter(move |sn, w, h| {
                let y = h - BAR_H as f32 + offset.get() as f32;
                let l = look.borrow();
                sn.append_color(&l.tint, &gtk::graphene::Rect::new(0.0, y, w, BAR_H as f32));
                sn.append_color(&l.stroke, &gtk::graphene::Rect::new(0.0, y, w, 1.0));
            });
        }
        let bar = gtk::CenterBox::new();
        bar.add_css_class("st-tbprev-bar");
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("st-tbprev");
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.set_child(Some(&base));
        overlay.add_overlay(&bar);
        overlay.set_can_target(true);
        {
            let (offset, bar) = (offset.clone(), bar.clone());
            overlay.connect_get_child_position(move |ov, w| {
                if w != bar.upcast_ref::<gtk::Widget>() {
                    return None;
                }
                Some(gdk::Rectangle::new(0, ov.height() - BAR_H + offset.get(), ov.width(), BAR_H))
            });
        }
        let root = Clamp::new(&overlay, 640);
        root.set_widget_name("taskbar-preview");
        let me = Rc::new(Preview {
            root,
            overlay: overlay.clone(),
            base,
            bar,
            offset,
            autohide: Cell::new(false),
            hover: Cell::new(false),
            left_aligned: Cell::new(false),
            look,
            clock: RefCell::new(None),
            extras: RefCell::new(Vec::new()),
            tray,
        });
        // .st-tbprev.is-autohide:hover: the hidden bar comes up while the pointer is on the preview
        let motion = gtk::EventControllerMotion::new();
        {
            let weak = Rc::downgrade(&me);
            motion.connect_enter(move |_, _, _| {
                if let Some(me) = weak.upgrade() {
                    me.hover.set(true);
                    me.place_bar();
                }
            });
        }
        {
            let weak = Rc::downgrade(&me);
            motion.connect_leave(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.hover.set(false);
                    me.place_bar();
                }
            });
        }
        overlay.add_controller(motion);
        {
            let weak = Rc::downgrade(&me);
            me.root.connect_width(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.fit();
                }
            });
        }
        {
            let weak = Rc::downgrade(&me);
            p.interval(1000, move || match weak.upgrade() {
                Some(me) => {
                    me.tick();
                    true
                }
                None => false,
            });
        }
        me.paint();
        me
    }

    fn place_bar(&self) {
        self.offset.set(if self.autohide.get() && !self.hover.get() { 36 } else { 0 });
        self.overlay.queue_allocate();
        self.base.queue_draw();
    }

    fn tick(&self) {
        let now = kit::format::now_ms();
        if let Some((tl, dl)) = self.clock.borrow().as_ref() {
            tl.set_label(&kit::format::time(now, common::setting_bool("taskbar.showSeconds", false), kit::ctx().setting("time.h24") != false));
            if let Some(dl) = dl {
                dl.set_label(&kit::format::date(now, kit::format::DateFormat::Short));
            }
        }
    }

    /// The mini bar is narrow (the search box takes most of it): show only the tray extras that fit, dropping
    /// the last icon first and the ^ last.
    fn fit(&self) {
        let extras = self.extras.borrow().clone();
        if extras.is_empty() {
            return;
        }
        for e in &extras {
            e.set_visible(true);
        }
        let avail = self.overlay.width() - 16;
        if avail <= 0 {
            return;
        }
        let nat = |w: Option<gtk::Widget>| w.map(|w| w.measure(gtk::Orientation::Horizontal, -1).1).unwrap_or(0);
        let fits = || {
            let (s, cen, e) = (nat(self.bar.start_widget()), nat(self.bar.center_widget()), nat(self.bar.end_widget()));
            if self.left_aligned.get() { s + e + 8 <= avail } else { cen + 2 * s.max(e) <= avail }
        };
        for e in extras.iter().rev() {
            if fits() {
                break;
            }
            e.set_visible(false);
        }
    }

    fn paint(&self) {
        let s = kit::ctx().settings();
        let tb = &s["taskbar"];
        let dark = kit::theme::dark();
        let accent_raw = s["theme"]["accent"].as_str().unwrap_or("#0078d4").to_owned();
        let (fill, _) = common::accent_for(&accent_raw, dark);
        let tb_accent = gdk::RGBA::parse(fill.as_str()).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0));
        self.base.set_color(Some(s["wallpaper"]["color"].as_str().and_then(|c| gdk::RGBA::parse(c).ok()).unwrap_or(gdk::RGBA::new(0.043, 0.165, 0.29, 1.0))));
        self.base.set_path(&common::resolve_wallpaper_path(&s, dark), 640);
        let left = tb["alignment"] == "left";
        self.left_aligned.set(left);
        self.autohide.set(tb["autoHide"] == true);
        // "Accentkleur weergeven op Start en taakbalk" only applies in dark mode, like Windows
        {
            let mut l = self.look.borrow_mut();
            l.tint = if s["theme"]["accentOnTaskbar"] == true && dark {
                let raw = gdk::RGBA::parse(accent_raw.as_str()).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0));
                mix(&raw, &gdk::RGBA::new(16.0 / 255.0, 16.0 / 255.0, 16.0 / 255.0, 0.8), 0.5)
            } else if dark {
                gdk::RGBA::new(28.0 / 255.0, 28.0 / 255.0, 28.0 / 255.0, 0.86)
            } else {
                gdk::RGBA::new(238.0 / 255.0, 238.0 / 255.0, 238.0 / 255.0, 0.86)
            };
            l.stroke = if dark { gdk::RGBA::new(1.0, 1.0, 1.0, 0.075) } else { gdk::RGBA::new(0.0, 0.0, 0.0, 0.075) };
        }

        // ---- the centre: Start, search, task view, three apps
        let center = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        center.set_valign(gtk::Align::Center);
        center.append(&tb_btn(&kit::icons::glyph("start-logo", 18)));
        match tb["search"].as_str().unwrap_or("") {
            mode @ ("box" | "iconLabel") => {
                let sb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                sb.add_css_class("st-tbprev-search");
                if mode == "box" {
                    sb.set_size_request(170, 30);
                } else {
                    sb.add_css_class("st-pill");
                }
                sb.set_valign(gtk::Align::Center);
                let g = kit::icons::glyph("search", 14);
                g.set_valign(gtk::Align::Center);
                sb.append(&g);
                let l = c::label(t("Zoeken", "Search"), "", false);
                l.set_valign(gtk::Align::Center);
                sb.append(&l);
                center.append(&sb);
            }
            "icon" => center.append(&tb_btn(&kit::icons::glyph("search", 18))),
            _ => {}
        }
        if tb["taskView"] == true {
            center.append(&tb_btn(&kit::icons::glyph("taskview", 18)));
        }
        for n in ["system-file-manager", "firefox", "preferences-system"] {
            let active = n == "preferences-system";
            let btn = tb_btn(&kit::icons::app_icon(n, 22));
            if active {
                btn.add_css_class("st-active");
            }
            let ov = gtk::Overlay::new();
            ov.set_child(Some(&btn));
            ov.add_overlay(&app_mark(active, tb_accent));
            center.append(&ov);
        }

        // ---- the left: widgets
        let left_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left_box.set_valign(gtk::Align::Center);
        if tb["widgets"] == true {
            let w = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            w.add_css_class("st-tbprev-widget");
            w.append(&kit::icons::glyph("widgets", 16));
            w.append(&c::label("14°", "", false));
            left_box.append(&w);
        }

        // ---- the notification area like Win11: ^, app icons, quick settings (wifi, volume), the clock
        let tray = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tray.add_css_class("st-tbprev-tray");
        tray.set_valign(gtk::Align::Center);
        let (chevron, icons) = if self.tray.supported.get() { tray_preview(&self.tray.items.borrow()) } else { (false, Vec::new()) };
        let mut extras: Vec<gtk::Widget> = Vec::new();
        if chevron {
            extras.push(kit::icons::glyph("chevron-up", 12).upcast());
        }
        for i in icons {
            extras.push(match i {
                Some(n) => kit::icons::app_icon(&n, 16).upcast(),
                None => kit::icons::glyph("app", 14).upcast(),
            });
        }
        for e in &extras {
            e.set_valign(gtk::Align::Center);
            tray.append(e);
        }
        tray.append(&kit::icons::glyph("wifi-3", 14));
        tray.append(&kit::icons::glyph("volume", 14));
        let clock = gtk::Box::new(gtk::Orientation::Vertical, 0);
        clock.add_css_class("st-tbprev-clock");
        clock.set_valign(gtk::Align::Center);
        let tl = c::label("", "w11-tabular", false);
        tl.set_xalign(1.0);
        clock.append(&tl);
        let dl = (s["time"]["showDate"] != false).then(|| {
            let dl = c::label("", "", false);
            dl.set_xalign(1.0);
            clock.append(&dl);
            dl
        });
        tray.append(&clock);
        *self.clock.borrow_mut() = Some((tl, dl));
        *self.extras.borrow_mut() = extras;

        self.bar.set_start_widget(None::<&gtk::Widget>);
        self.bar.set_center_widget(None::<&gtk::Widget>);
        self.bar.set_end_widget(None::<&gtk::Widget>);
        if left {
            self.bar.set_start_widget(Some(&center));
        } else {
            self.bar.set_start_widget(Some(&left_box));
            self.bar.set_center_widget(Some(&center));
        }
        self.bar.set_end_widget(Some(&tray));
        self.place_bar();
        self.tick();
        self.fit();
    }
}

/// Scroll a section into view once the page is in the window (taskbar.js reveal; scroll-margin-top 12 px).
fn reveal(p: &Page, w: gtk::Widget) {
    let pp = p.clone();
    p.timeout(300, move || {
        let tries = Cell::new(0);
        pp.interval(16, move || {
            tries.set(tries.get() + 1);
            let more = tries.get() < 120;
            let Some(sw) = w.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() else { return more };
            if !w.is_mapped() || w.height() <= 0 {
                return more;
            }
            let Some(content) = sw.child().map(|c| c.downcast_ref::<gtk::Viewport>().and_then(|v| v.child()).unwrap_or(c)) else { return more };
            let Some(pt) = w.compute_point(&content, &gtk::graphene::Point::new(0.0, 0.0)) else { return more };
            sw.vadjustment().set_value((pt.y() as f64 - 12.0).max(0.0));
            false
        });
    });
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let open_tray = p.param("section").is_some_and(|s| TRAY_SECTIONS.contains(&s.to_lowercase().as_str()));
        let (supported, items) = load_tray(&p).await;
        if !p.alive() {
            return Content::new();
        }
        let tray = Rc::new(Tray { supported: Cell::new(supported), items: RefCell::new(items) });

        // ---- Taakbalkitems
        let search = common::setting_dropdown(
            &p,
            "taskbar.search",
            vec![
                (json!("hidden"), t("Verbergen", "Hide").into()),
                (json!("icon"), t("Alleen pictogram zoeken", "Search icon only").into()),
                (json!("iconLabel"), t("Pictogram en label zoeken", "Search icon and label").into()),
                (json!("box"), t("Zoekvak", "Search box").into()),
            ],
            Some(json!("box")),
        );
        search.widget().set_widget_name("taskbar-search");
        let search_card = SettingsCard::new("search", t("Zoeken", "Search"), "");
        search_card.set_control(&common::sized(search.widget(), 240));
        let (tv_card, tv_sw) = common::toggle_card(&p, "taskview", t("Taakweergave", "Task view"), "", "taskbar.taskView", false);
        tv_sw.switch().set_widget_name("taskbar-taskview");
        let (wd_card, wd_sw) = common::toggle_card(&p, "widgets", t("Widgets", "Widgets"), "", "taskbar.widgets", false);
        wd_sw.switch().set_widget_name("taskbar-widgets");
        let items = SettingsExpander::new("taskbar", t("Taakbalkitems", "Taskbar items"), t("Pictogrammen op de taakbalk weergeven of verbergen", "Show or hide icons on the taskbar"));
        items.set_expanded(true);
        items.add(search_card.widget());
        items.add(tv_card.widget());
        items.add(wd_card.widget());

        let section = TraySection::new(&p, tray.clone(), open_tray);

        // ---- Gedrag van de taakbalk
        let align = common::setting_dropdown(
            &p,
            "taskbar.alignment",
            vec![(json!("left"), t("Links", "Left").into()), (json!("center"), t("Gecentreerd", "Centre").into())],
            Some(json!("center")),
        );
        align.widget().set_widget_name("taskbar-alignment");
        let align_card = SettingsCard::new("", t("Uitlijning van de taakbalk", "Taskbar alignment"), "");
        align_card.set_control(&common::sized(align.widget(), 180));
        let (hide_card, hide_sw) = common::toggle_card(
            &p,
            "",
            t("De taakbalk automatisch verbergen", "Automatically hide the taskbar"),
            t("De taakbalk verschijnt als je de muis naar de onderkant van het scherm beweegt", "The taskbar appears when you move the mouse to the bottom of the screen"),
            "taskbar.autoHide",
            false,
        );
        hide_sw.switch().set_widget_name("taskbar-autohide");
        let seconds_on = || common::setting_bool("taskbar.showSeconds", false);
        let secs = ToggleSwitch::new(seconds_on());
        secs.switch().set_widget_name("taskbar-seconds");
        {
            let (pp, sw) = (p.clone(), secs.clone());
            secs.connect_toggled(move |on| {
                let wait = pp.call("settings.patch", json!({ "patch": { "taskbar": { "showSeconds": on }, "time": { "showSeconds": on } } }));
                let (p2, sw) = (pp.clone(), sw.clone());
                pp.spawn(async move {
                    if let Err(e) = wait.await {
                        sw.set_active(!on);
                        p2.error(&e, None);
                    }
                });
            });
        }
        {
            let sw = secs.clone();
            p.on_settings(move |_, _| sw.set_active(seconds_on()));
        }
        let secs_card = SettingsCard::new("", t("Seconden weergeven in de klok van het systeemvak", "Show seconds in system tray clock"), t("Gebruikt iets meer stroom", "Uses a little more power"));
        secs_card.set_control(secs.widget());
        let behavior = SettingsExpander::new("settings", t("Gedrag van de taakbalk", "Taskbar behaviours"), t("Uitlijning, automatisch verbergen, klok", "Alignment, auto-hide, clock"));
        behavior.set_expanded(true);
        behavior.add(align_card.widget());
        behavior.add(hide_card.widget());
        behavior.add(secs_card.widget());

        let preview = Preview::new(&p, tray.clone());
        {
            let (sec, prev) = (section.clone(), preview.clone());
            p.on_settings(move |_, path| {
                if path.is_none_or(|p| p.starts_with("taskbar")) {
                    sec.paint();
                }
                prev.paint();
            });
        }
        // the live notification area: icons come and go, the host may switch role (watcher, host, none)
        {
            let (tray, sec, prev) = (tray.clone(), section.clone(), preview.clone());
            p.on("tray.changed", move |d| {
                *tray.items.borrow_mut() = d["items"].as_array().cloned().unwrap_or_default();
                sec.paint();
                prev.paint();
            });
        }
        {
            let (pp, tray, sec, prev) = (p.clone(), tray.clone(), section.clone(), preview.clone());
            p.on("tray.status", move |d| {
                let was = tray.supported.get();
                tray.supported.set(d["supported"] != false);
                if tray.supported.get() && !was {
                    let (p2, tray, sec, prev) = (pp.clone(), tray.clone(), sec.clone(), prev.clone());
                    pp.spawn(async move {
                        let (supported, items) = load_tray(&p2).await;
                        if !p2.alive() {
                            return;
                        }
                        tray.supported.set(supported);
                        *tray.items.borrow_mut() = items;
                        sec.paint();
                        prev.paint();
                    });
                } else {
                    sec.paint();
                    prev.paint();
                }
            });
        }
        if open_tray {
            reveal(&p, section.exp.widget().clone().upcast());
        }

        let group = SettingsGroup::new("", "");
        group.add(items.widget());
        group.add(section.exp.widget());
        group.add(behavior.widget());
        vec![preview.root.clone().upcast(), group.widget().clone().upcast()]
    })
}
