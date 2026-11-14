//! Win11 GNOME Default apps in GTK 4 (../../APPS.md): last-webkit:vendor/ui/js/apps/defaultapps/main.js and
//! css/defaultapps.css on the kit. A searchable list of file types and link schemes, 100 per page
//! (apps.mimeTypes), each with its default app; a click asks "Een app kiezen voor ..." with the apps that can open
//! it (apps.forType), and "Als standaard instellen" sets one (apps.setDefaultForType). The list follows
//! apps.changed, the window getting the focus and the language. One process: a relaunch presents the window and,
//! when it names a search (--params {"query"|"extension"|"mime": ...}), searches for it (the WebKit app's
//! app.navigate).
mod clamp;

use clamp::Clamp;
use kit::controls::{
    self as c, Chevron, Dialog, DialogResult, InfoBar, SettingsCard, SettingsGroup, Severity, Toast, Variant,
};
use kit::frame::{AppWindow, WindowOptions};
use kit::prelude::*;
use kit::t;
use serde_json::Map;
use std::cell::RefCell;
use std::rc::Rc;

const USAGE: &str = "win11-gtk-defaultapps [--params JSON]

  --params '{\"query\":\"pdf\"}'   open with this search; \"extension\" or \"mime\" work the same way
  A second launch is handed to the running window.";

const SPEC: kit::AppSpec =
    kit::AppSpec::new("defaultapps", "io.github.win11shell.DefaultApps", "Standaard-apps", "Default apps")
        .icon("apps")
        .usage(USAGE);

/// apps.mimeTypes page size (main.js PAGE_SIZE).
const PAGE_SIZE: u64 = 100;
/// `.da-default` width at > 700, <= 700 and <= 500 px (the media queries of defaultapps.css).
const DEFAULT_WIDTH: [i32; 3] = [230, 160, 100];

fn main() -> glib::ExitCode {
    let running: Rc<RefCell<Option<Rc<App>>>> = Rc::default();
    kit::run(SPEC, move |ctx, args, first| {
        let params = params(&args);
        if !first {
            let app = running.borrow().clone();
            if let Some(app) = app {
                app.navigate(&params);
                app.win.present();
            }
            return;
        }
        kit::theme::install_app_css(include_str!("../css/defaultapps.css"));
        let app = App::new(ctx, &params);
        // No initial focus, as the page has none: GTK would put it (with the accent line and the text selected) in
        // the search box.
        let text = app.search.text_widget().clone();
        app.win.window().connect_map(move |w| {
            let (w, text) = (w.clone(), text.clone());
            glib::idle_add_local_once(move || {
                GtkWindowExt::set_focus(&w, None::<&gtk::Widget>);
                text.select_region(-1, -1);
            });
        });
        app.win.present();
        app.start();
        *running.borrow_mut() = Some(app);
    })
}

/// The launch arguments as the WebKit app got them in its URL query (`git show last-webkit:host/src/lib.rs`,
/// run_inner): `--params JSON` gives its keys (strings as they are, other values as JSON text), `--tab-ticket T`
/// gives tabTicket, otherwise the first argument is `path` and the others `tabs`.
fn params(args: &[String]) -> Map<String, Value> {
    let mut args: Vec<&str> =
        args.iter().map(String::as_str).filter(|a| !matches!(*a, "--dry-run" | "--http-only")).collect();
    if args.first() == Some(&"--app") {
        args.drain(..args.len().min(2));
    }
    let mut out = Map::new();
    match args.first().copied() {
        Some("--tab-ticket") => {
            if let Some(ticket) = args.get(1) {
                out.insert("tabTicket".into(), json!(ticket));
            }
        }
        Some("--params") => {
            if let Some(Value::Object(given)) = args.get(1).and_then(|raw| serde_json::from_str::<Value>(raw).ok()) {
                for (key, value) in given {
                    let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                    out.insert(key, Value::String(text));
                }
            }
        }
        Some(path) => {
            out.insert("path".into(), json!(path));
            if args.len() > 1 {
                out.insert("tabs".into(), json!(serde_json::to_string(&args[1..]).unwrap_or_default()));
            }
        }
        None => {}
    }
    out
}

/// `params.query || params.extension || params.mime || ''`.
fn search_param(params: &Map<String, Value>) -> String {
    ["query", "extension", "mime"]
        .iter()
        .filter_map(|k| params.get(*k).and_then(Value::as_str))
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// i18n.js fill: `{name}` replaced by its value.
fn fill(template: &str, params: &[(&str, &str)]) -> String {
    let mut s = template.to_owned();
    for (key, value) in params {
        s = s.replace(&format!("{{{key}}}"), value);
    }
    s
}

/// The first descendant (or the widget itself) with this CSS class.
fn find_class(w: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if w.has_css_class(class) {
        return Some(w.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(found) = find_class(&c, class) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// The first label inside a widget.
fn find_label(w: &gtk::Widget) -> Option<gtk::Label> {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        return Some(l.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(found) = find_label(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// A text button with a glyph whose text follows the language.
fn glyph_button(glyph: &str, nl: &'static str, en: &'static str) -> gtk::Button {
    let b = c::button_with_glyph(glyph, t(nl, en), Variant::Standard);
    if let Some(l) = find_label(b.upcast_ref()) {
        kit::i18n::bind(&l, move |l| l.set_label(t(nl, en)));
    }
    kit::i18n::bind(&b, move |b| b.update_property(&[gtk::accessible::Property::Label(t(nl, en))]));
    b
}

/// A label that follows the language.
fn text(class: &str, wrap: bool, nl: &'static str, en: &'static str) -> gtk::Label {
    let l = c::label(t(nl, en), class, wrap);
    kit::i18n::bind(&l, move |l| l.set_label(t(nl, en)));
    l
}

#[derive(Default)]
struct State {
    query: String,
    offset: u64,
    /// The last apps.mimeTypes answer {items, total, offset, limit}.
    snapshot: Option<Value>,
    /// The last load's error text (shown instead of the list).
    failure: Option<String>,
    serial: u64,
    /// An apps.forType request is under way.
    selecting: bool,
    /// The open "Een app kiezen" dialog (its number, the dialog).
    dialog: Option<(u64, Dialog)>,
    dialogs: u64,
    /// 0: wider than 700 px, 1: <= 700, 2: <= 500.
    level: usize,
}

struct App {
    ctx: Rc<kit::Ctx>,
    win: AppWindow,
    page: gtk::Box,
    search: c::SearchBox,
    list: gtk::Box,
    pagination: gtk::Box,
    range: gtk::Label,
    buttons: gtk::Box,
    previous: gtk::Button,
    next: gtk::Button,
    state: RefCell<State>,
}

impl App {
    fn new(ctx: &Rc<kit::Ctx>, params: &Map<String, Value>) -> Rc<App> {
        let win = AppWindow::new(WindowOptions { width: 1100, height: 740, ..Default::default() });
        let query = search_param(params);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
        page.add_css_class("defaultapps-page");
        let header = gtk::Box::new(gtk::Orientation::Vertical, 8);
        header.append(&text("defaultapps-title", true, "Standaard-apps", "Default apps"));
        header.append(&text(
            "defaultapps-intro",
            true,
            "Kies welke app een bestandstype of koppeling opent.",
            "Choose which app opens a file type or link.",
        ));
        page.append(&header);

        let search = c::SearchBox::new(Some(t("Zoek een extensie of bestandstype", "Search for an extension or file type")));
        search.text_widget().set_widget_name("defaultapps-search");
        kit::i18n::bind(search.text_widget(), |w| {
            let p = t("Zoek een extensie of bestandstype", "Search for an extension or file type");
            w.set_placeholder_text(Some(p));
            w.update_property(&[gtk::accessible::Property::Label(p)]);
        });
        search.set_text(&query);
        search.set_debounce(180);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.append(search.widget());
        column.append(&text(
            "defaultapps-example",
            true,
            "Bijvoorbeeld .pdf, .jpg of text/plain",
            "For example .pdf, .jpg or text/plain",
        ));
        page.append(&Clamp::new(&column, 560, false));

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("defaultapps-list");
        list.set_widget_name("defaultapps-list");
        list.set_size_request(-1, 180);
        page.append(&list);

        let pagination = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        pagination.add_css_class("defaultapps-pagination");
        kit::i18n::bind(&pagination, |p| p.update_property(&[gtk::accessible::Property::Label(t("Standaard-apps", "Default apps"))]));
        let range = c::label("", "defaultapps-range", false);
        range.set_hexpand(true);
        range.set_valign(gtk::Align::Center);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_valign(gtk::Align::Center);
        let previous = glyph_button("chevron-left", "Vorige", "Previous");
        previous.set_widget_name("defaultapps-previous");
        let next = glyph_button("chevron-right", "Volgende", "Next");
        next.set_widget_name("defaultapps-next");
        buttons.append(&previous);
        buttons.append(&next);
        pagination.append(&range);
        pagination.append(&buttons);
        page.append(&pagination);

        let outer = Clamp::new(&page, 1120, true);
        win.set_content(&c::scrolled(&outer));

        let app = Rc::new(App {
            ctx: ctx.clone(),
            win,
            page,
            search,
            list,
            pagination,
            range,
            buttons,
            previous,
            next,
            state: RefCell::new(State { query, ..Default::default() }),
        });

        let weak = Rc::downgrade(&app);
        app.search.connect_changed(move |value| {
            if let Some(app) = weak.upgrade() {
                app.search_for(value);
            }
        });
        let weak = Rc::downgrade(&app);
        app.search.connect_submit(move |value| {
            if let Some(app) = weak.upgrade() {
                app.search_for(value);
            }
        });
        let weak = Rc::downgrade(&app);
        app.previous.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                {
                    let mut s = app.state.borrow_mut();
                    s.offset = s.offset.saturating_sub(PAGE_SIZE);
                }
                app.load();
            }
        });
        let weak = Rc::downgrade(&app);
        app.next.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().offset += PAGE_SIZE;
                app.load();
            }
        });
        let weak = Rc::downgrade(&app);
        outer.connect_width(move |width| {
            if let Some(app) = weak.upgrade() {
                app.set_width(width);
            }
        });
        let weak = Rc::downgrade(&app);
        ctx.on("apps.changed", move |_, _| {
            if let Some(app) = weak.upgrade()
                && !app.dialog_open()
            {
                app.load();
            }
        });
        let weak = Rc::downgrade(&app);
        app.win.window().connect_is_active_notify(move |w| {
            if w.is_active()
                && let Some(app) = weak.upgrade()
                && !app.dialog_open()
            {
                app.load();
            }
        });
        let weak = Rc::downgrade(&app);
        kit::i18n::on_change(move || {
            if let Some(app) = weak.upgrade() {
                app.close_dialog();
                app.load();
            }
        });
        app
    }

    fn start(self: &Rc<Self>) {
        self.render();
        self.load();
    }

    /// A relaunch (main.js frame.onAppNavigate): a new search when it names one.
    fn navigate(self: &Rc<Self>, params: &Map<String, Value>) {
        if !["query", "extension", "mime"].iter().any(|k| params.contains_key(*k)) {
            return;
        }
        let query = search_param(params);
        self.search.set_text(&query);
        {
            let mut s = self.state.borrow_mut();
            s.query = query;
            s.offset = 0;
        }
        self.load();
    }

    fn search_for(self: &Rc<Self>, value: &str) {
        {
            let mut s = self.state.borrow_mut();
            s.query = value.to_owned();
            s.offset = 0;
        }
        self.load();
    }

    fn dialog_open(&self) -> bool {
        self.state.borrow().dialog.is_some()
    }

    fn close_dialog(&self) {
        let dialog = self.state.borrow_mut().dialog.take();
        if let Some((_, d)) = dialog {
            d.close(DialogResult::Close);
        }
    }

    /// The media queries: padding and the default column's width at <= 700 px, the pagination stacked at <= 500.
    fn set_width(self: &Rc<Self>, width: i32) {
        let level = if width <= 500 {
            2
        } else if width <= 700 {
            1
        } else {
            0
        };
        let rerender = {
            let mut s = self.state.borrow_mut();
            if s.level == level {
                return;
            }
            s.level = level;
            s.snapshot.is_some() && s.failure.is_none()
        };
        if level >= 1 {
            self.page.add_css_class("defaultapps-narrow");
        } else {
            self.page.remove_css_class("defaultapps-narrow");
        }
        let stacked = level == 2;
        self.pagination.set_orientation(if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        self.buttons.set_halign(if stacked { gtk::Align::Start } else { gtk::Align::Fill });
        if rerender {
            self.render();
        }
    }

    fn set_busy(&self, busy: bool) {
        self.list.set_cursor_from_name(if busy { Some("progress") } else { None });
        self.list.update_state(&[gtk::accessible::State::Busy(busy)]);
    }

    fn load(self: &Rc<Self>) {
        let (request, args) = {
            let mut s = self.state.borrow_mut();
            s.serial += 1;
            (s.serial, json!({"query": s.query, "offset": s.offset, "limit": PAGE_SIZE}))
        };
        self.set_busy(true);
        let weak = Rc::downgrade(self);
        self.ctx.call("apps.mimeTypes", args, move |result| {
            let Some(app) = weak.upgrade() else { return };
            let again = {
                let mut s = app.state.borrow_mut();
                if request != s.serial {
                    return;
                }
                match result {
                    Ok(value) => {
                        let empty = value["items"].as_array().is_none_or(|a| a.is_empty());
                        let total = value["total"].as_u64().unwrap_or(0);
                        s.snapshot = Some(value);
                        s.failure = None;
                        // Past the end (the list shrank): the last page instead.
                        if s.offset > 0 && empty && total > 0 {
                            s.offset = (total - 1) / PAGE_SIZE * PAGE_SIZE;
                            true
                        } else {
                            false
                        }
                    }
                    Err(e) => {
                        s.failure = Some(e.message);
                        false
                    }
                }
            };
            if again {
                app.load();
            } else {
                app.render();
            }
        });
    }

    fn render(self: &Rc<Self>) {
        c::clear(&self.list);
        let s = self.state.borrow();
        self.set_busy(s.snapshot.is_none() && s.failure.is_none());
        let items: Vec<Value> =
            s.snapshot.as_ref().and_then(|v| v["items"].as_array()).cloned().unwrap_or_default();
        if let Some(message) = &s.failure {
            let retry = c::button(t("Opnieuw proberen", "Try again"), Variant::Standard);
            retry.set_widget_name("defaultapps-retry");
            let weak = Rc::downgrade(self);
            retry.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.load();
                }
            });
            self.list.append(&c::empty_state(
                "warning",
                t("Standaard-apps konden niet worden geladen", "Default apps couldn’t be loaded"),
                message,
                Some(retry.upcast_ref::<gtk::Widget>()),
            ));
        } else if s.snapshot.is_none() {
            let loading = gtk::Box::new(gtk::Orientation::Vertical, 16);
            loading.add_css_class("defaultapps-loading");
            loading.set_halign(gtk::Align::Center);
            loading.append(c::ProgressRing::new(28, None).widget());
            let l = c::label(t("Bestandstypen laden…", "Loading file types…"), "", false);
            l.set_xalign(0.5);
            loading.append(&l);
            self.list.append(&loading);
        } else if items.is_empty() {
            self.list.append(&c::empty_state(
                "search",
                t("Geen bestandstypen gevonden", "No file types found"),
                t("Probeer een andere extensie of zoekterm.", "Try another extension or search term."),
                None,
            ));
        } else {
            let group = SettingsGroup::new("", "");
            let width = DEFAULT_WIDTH[s.level.min(2)];
            for item in &items {
                group.add(&self.type_card(item, width));
            }
            self.list.append(group.widget());
        }
        let total = s.snapshot.as_ref().and_then(|v| v["total"].as_u64()).unwrap_or(0);
        let shown = items.len() as u64;
        if s.snapshot.is_some() {
            let from = if total > 0 { s.offset + 1 } else { 0 };
            let to = (s.offset + shown).min(total);
            self.range.set_label(&fill(
                t("{from}–{to} van {total} bestandstypen", "{from}–{to} of {total} file types"),
                &[("from", &from.to_string()), ("to", &to.to_string()), ("total", &total.to_string())],
            ));
        } else {
            self.range.set_label("");
        }
        self.previous.set_sensitive(s.snapshot.is_some() && s.offset > 0);
        self.next.set_sensitive(s.snapshot.is_some() && s.offset + shown < total);
    }

    /// One file type: its extensions (or name, or MIME type), its name and MIME type, and its default app.
    fn type_card(self: &Rc<Self>, item: &Value, width: i32) -> gtk::Widget {
        let mime = item["mime"].as_str().unwrap_or("").to_owned();
        let type_name = item["name"].as_str().unwrap_or("");
        let extensions: Vec<&str> = item["extensions"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).take(5).collect())
            .unwrap_or_default();
        let name = if !extensions.is_empty() {
            extensions.join(", ")
        } else if !type_name.is_empty() {
            type_name.to_owned()
        } else {
            mime.clone()
        };
        let current = item["default"].as_object();
        let current_name = current.and_then(|o| o.get("name")).and_then(Value::as_str).filter(|s| !s.is_empty());

        let card = SettingsCard::clickable("", &name, type_name, Chevron::Right);
        let icon = kit::icons::file_icon(&mime, 32);
        // .da-type .fl-sc__icon: a 36 px box, the 32 px icon centred in it.
        icon.set_size_request(36, 36);
        card.set_icon_widget(&icon);
        // The MIME type under the type's name (.da-mime).
        if let Some(text_box) =
            find_class(card.widget(), "w11-sc-desc").and_then(|d| d.parent()).and_downcast::<gtk::Box>()
        {
            text_box.append(&c::label(&mime, "defaultapps-mime", true));
        }

        let control = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        control.add_css_class("defaultapps-default");
        let app_icon = match current {
            Some(cur) => {
                let icon = cur
                    .get("icon")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| cur.get("id").and_then(Value::as_str))
                    .unwrap_or("");
                kit::icons::app_icon(icon, 24)
            }
            None => kit::icons::file_icon(&mime, 24),
        };
        control.append(&app_icon);
        control.append(&c::label(
            current_name.unwrap_or(t("Kies een standaard-app", "Choose a default app")),
            "defaultapps-default-name",
            false,
        ));
        let column = Clamp::new(&control, width, false);
        column.set_size_request(width, -1);
        card.set_control(&column);

        let w = card.widget().clone();
        w.add_css_class("defaultapps-type");
        w.set_widget_name(&format!("defaultapps-type-{mime}"));
        let label = format!("{name}: {}", current_name.unwrap_or(t("Geen standaard-app", "No default app")));
        w.update_property(&[gtk::accessible::Property::Label(&label)]);
        let weak = Rc::downgrade(self);
        let item = item.clone();
        card.connect_clicked(move || {
            if let Some(app) = weak.upgrade() {
                app.choose(&item);
            }
        });
        w
    }

    fn choose(self: &Rc<Self>, item: &Value) {
        {
            let mut s = self.state.borrow_mut();
            if s.selecting {
                return;
            }
            s.selecting = true;
        }
        let weak = Rc::downgrade(self);
        let mime = item["mime"].as_str().unwrap_or("").to_owned();
        self.ctx.call("apps.forType", json!({ "mime": mime }), move |result| {
            let Some(app) = weak.upgrade() else { return };
            app.state.borrow_mut().selecting = false;
            match result {
                Ok(detail) => app.show_choices(detail),
                Err(e) => {
                    Toast::new(t("Standaard-apps konden niet worden geladen", "Default apps couldn’t be loaded"))
                        .message(&e.message)
                        .severity(Severity::Error)
                        .show(app.win.window());
                }
            }
        });
    }

    /// "Een app kiezen voor .pdf": the apps for the type as radio rows, "Als standaard instellen" sets the chosen one.
    fn show_choices(self: &Rc<Self>, detail: Value) {
        let mime = detail["mime"].as_str().unwrap_or("").to_owned();
        let default_id = detail["default"]["id"].as_str().unwrap_or("").to_owned();
        let apps: Vec<Value> = detail["apps"].as_array().cloned().unwrap_or_default();
        let type_label = detail["extensions"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&mime)
            .to_owned();
        let has_apps = !apps.is_empty();
        let selected = Rc::new(RefCell::new(default_id.clone()));
        let handle: Rc<RefCell<Option<Dialog>>> = Rc::default();
        let rows: Rc<RefCell<Vec<(String, gtk::CheckButton)>>> = Rc::default();
        let update: Rc<dyn Fn()> = {
            let (selected, handle, rows, default_id) = (selected.clone(), handle.clone(), rows.clone(), default_id.clone());
            Rc::new(move || {
                let sel = selected.borrow().clone();
                for (id, row) in rows.borrow().iter() {
                    if *id == sel {
                        row.add_css_class("defaultapps-selected");
                    } else {
                        row.remove_css_class("defaultapps-selected");
                    }
                }
                if let Some(d) = handle.borrow().as_ref() {
                    d.set_primary_enabled(!sel.is_empty() && sel != default_id);
                }
            })
        };

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("defaultapps-dialog-content");
        let kind = gtk::Box::new(gtk::Orientation::Vertical, 0);
        kind.add_css_class("defaultapps-dialog-type");
        kind.append(&c::label(detail["name"].as_str().unwrap_or(""), "", true));
        kind.append(&c::label(&mime, "defaultapps-mime", true));
        content.append(&kind);
        if has_apps {
            let relevant = t("Apps die dit bestandstype kunnen openen", "Apps that can open this file type");
            content.append(&c::label(relevant, "defaultapps-dialog-intro", true));
            let choices = gtk::Box::new(gtk::Orientation::Vertical, 4);
            choices.add_css_class("defaultapps-choices");
            choices.update_property(&[gtk::accessible::Property::Label(relevant)]);
            let mut first: Option<gtk::CheckButton> = None;
            for a in &apps {
                let id = a["id"].as_str().unwrap_or("").to_owned();
                let b = gtk::CheckButton::new();
                b.add_css_class("w11-radio");
                b.add_css_class("defaultapps-choice");
                match &first {
                    Some(f) => b.set_group(Some(f)),
                    None => first = Some(b.clone()),
                }
                b.set_active(!id.is_empty() && id == default_id);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                let icon = a["icon"].as_str().filter(|s| !s.is_empty()).unwrap_or(&id);
                row.append(&kit::icons::app_icon(icon, 32));
                let texts = gtk::Box::new(gtk::Orientation::Vertical, 3);
                texts.set_valign(gtk::Align::Center);
                texts.append(&c::label(a["name"].as_str().unwrap_or(""), "defaultapps-choice-name", false));
                if a["isDefault"] == true {
                    texts.append(&c::label(t("Huidige standaard", "Current default"), "defaultapps-choice-current", false));
                }
                row.append(&texts);
                b.set_child(Some(&row));
                b.set_widget_name(&format!("defaultapps-choice-{id}"));
                let (selected, update, value) = (selected.clone(), update.clone(), id.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        *selected.borrow_mut() = value.clone();
                        update();
                    }
                });
                choices.append(&b);
                rows.borrow_mut().push((id, b));
            }
            let scroller = gtk::ScrolledWindow::new();
            scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
            scroller.set_propagate_natural_height(true);
            scroller.set_max_content_height(340);
            scroller.set_child(Some(&choices));
            content.append(&scroller);
        } else {
            content.append(&c::empty_state(
                "apps",
                t("Geen geschikte apps gevonden", "No suitable apps found"),
                t(
                    "Installeer een app die dit bestandstype ondersteunt en probeer het opnieuw.",
                    "Install an app that supports this file type and try again.",
                ),
                None,
            ));
        }
        let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status.add_css_class("defaultapps-dialog-status");
        status.set_visible(false);
        content.append(&status);

        let mut dialog = Dialog::new(&fill(t("Een app kiezen voor {type}", "Choose an app for {type}"), &[("type", &type_label)]))
            // The page's dialog is 520 px wide (border-box): long names wrap or end in "..." instead of widening it.
            .body(&Clamp::new(&content, 520 - 2 - 48, false))
            .close_label(t("Annuleren", "Cancel"))
            .width(520);
        if has_apps {
            dialog = dialog.primary(t("Als standaard instellen", "Set default"));
        }
        let weak = Rc::downgrade(self);
        let chosen = selected.clone();
        dialog = dialog.on_primary(move |d| {
            let id = chosen.borrow().clone();
            if id.is_empty() || id == default_id {
                return false;
            }
            let Some(app) = weak.upgrade() else { return false };
            c::clear(&status);
            status.set_visible(false);
            d.set_busy(true);
            let name = apps
                .iter()
                .find(|a| a["id"] == id.as_str())
                .and_then(|a| a["name"].as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&id)
                .to_owned();
            let (d, status, type_label, weak) = (d.clone(), status.clone(), type_label.clone(), Rc::downgrade(&app));
            app.ctx.call("apps.setDefaultForType", json!({ "mime": mime, "id": id }), move |result| match result {
                Ok(result) => {
                    if let Some(app) = weak.upgrade() {
                        if result["effective"] == false {
                            let files: Vec<&str> = result["overriddenBy"]
                                .as_array()
                                .map(|a| a.iter().filter_map(Value::as_str).collect())
                                .unwrap_or_default();
                            let message = if files.is_empty() {
                                String::new()
                            } else {
                                fill(
                                    t("Deze bestanden hebben voorrang: {files}", "These files take precedence: {files}"),
                                    &[("files", &files.join(", "))],
                                )
                            };
                            Toast::new(t(
                                "De app is opgeslagen, maar een andere bureaubladinstelling heeft voorrang.",
                                "The app was saved, but another desktop setting takes precedence.",
                            ))
                            .message(&message)
                            .severity(Severity::Warning)
                            .timeout(10000)
                            .show(app.win.window());
                        } else {
                            Toast::new(&fill(
                                t("{app} is nu de standaard voor {type}", "{app} is now the default for {type}"),
                                &[("app", &name), ("type", &type_label)],
                            ))
                            .severity(Severity::Success)
                            .timeout(3500)
                            .show(app.win.window());
                        }
                        app.load();
                    }
                    d.close(DialogResult::Primary);
                }
                Err(e) => {
                    d.set_busy(false);
                    let bar = InfoBar::new(
                        Severity::Error,
                        t("De standaard-app kon niet worden gewijzigd.", "The default app couldn’t be changed."),
                        &e.message,
                    );
                    status.append(bar.widget());
                    status.set_visible(true);
                }
            });
            false
        });

        let number = {
            let mut s = self.state.borrow_mut();
            s.dialogs += 1;
            s.dialogs
        };
        let weak = Rc::downgrade(self);
        let (closing_handle, closing_rows) = (handle.clone(), rows.clone());
        dialog.show(self.win.window(), move |_| {
            // Break the row <-> handler cycles of this dialog.
            closing_handle.borrow_mut().take();
            closing_rows.borrow_mut().clear();
            if let Some(app) = weak.upgrade() {
                let mut s = app.state.borrow_mut();
                if s.dialog.as_ref().is_some_and(|(n, _)| *n == number) {
                    s.dialog = None;
                }
            }
        });
        if dialog.is_open() {
            *handle.borrow_mut() = Some(dialog.clone());
            self.state.borrow_mut().dialog = Some((number, dialog));
            update();
        }
    }
}
