//! The file picker (vendor/ui/js/apps/explorer/picker.js, css/picker.css): the Explorer as the Open / Save /
//! Choose folder dialog of the desktop portal (win11-file-portal, host/src/portal.rs) and of anyone who runs it.
//! The regular Explorer is reused; the title bar shows the dialog's title instead of the tabs, and a footer under
//! the status bar holds the file name box, the file type filter (the portal's glob and MIME rules), the portal's
//! choices (check boxes and drop-downs), "Alleen-lezen openen", the error line and the accept / Annuleren buttons.
//! Opening a file chooses it; a folder typed or chosen in Open or Save mode is opened instead; Save asks before
//! it replaces an existing file; Escape and the close button cancel.
//!
//! # Running it
//!
//! A picker is its own process with its own GApplication id (io.github.win11shell.FilePicker.p<pid>), so it is
//! never handed to a running Explorer and its stdin and stdout are its own. Its windows have the Wayland app id
//! io.github.win11shell.FilePicker, like the WebKit picker's.
//!
//! - `win11-gtk-explorer --picker-stdio` (the portal): the first line on stdin is the request,
//!   `{"picker": OPTIONS, "path": "/start/folder", "parent": "wayland:HANDLE"}`. The picker writes one JSON line
//!   to stdout when the user decides: `{"cmd": "picker-submit", "path", "paths", "filterIndex", "choices":
//!   [[id, value]], "writable"}` or `{"cmd": "close"}`. After a submit it waits for the caller's line on stdin:
//!   `{"ok": true}` closes the picker, `{"error": "text"}` shows the text in the footer and keeps it open (the
//!   host's own check, picker.rs `selection`, as the WebKit picker's `win11:picker-error`). End of stdin closes
//!   the picker without an answer.
//! - `win11-gtk-explorer --params '{"picker": OPTIONS, "path": ...}'` (screenshots, tests): the same window; the
//!   answer line goes to stdout and the window closes at once.
//!
//! OPTIONS are picker.js's: folder, save (or action "save"), multiple, filters [{name, patterns, conditions
//! [{type 0 glob | 1 MIME, value}], caseSensitive}], currentFilter, name (or fileName), choices [{id, label,
//! options [{id, label}], selected}], acceptLabel, title, allowReadOnly, writable, modal.
//!
//! A "parent" of the form "wayland:HANDLE" (xdg-foreign) makes the window transient for the caller's window
//! (and modal unless `modal` is false); X11 parents are not supported by GTK 4 for foreign windows and are
//! ignored, as an invalid parent is by the GTK reference backend.
use crate::explorer::{Explorer, NavOpts, Params, Picker, WeakExplorer};
use crate::model::Loc;
use crate::util::{self, Item, flag, st};
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk::{gdk, glib};
use kit::controls::{self, Dropdown, DropdownOption, Variant};
use kit::t;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, Read, Write};
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The Wayland app id of a picker window (host/src/picker.rs gave the WebKit picker the same one).
const PICKER_APP_ID: &str = "io.github.win11shell.FilePicker";
/// The request line on stdin is at most this long.
const REQUEST_MAX: u64 = 8 << 20;

const CSS: &str = include_str!("../css/picker.css");

unsafe extern "C" {
    // gdk/wayland/gdkwayland.h (GTK 4): parent a toplevel to a surface another client exported (xdg-foreign).
    fn gdk_wayland_toplevel_set_transient_for_exported(
        toplevel: *mut gdk::ffi::GdkToplevel,
        parent_handle_str: *const libc::c_char,
    ) -> glib::ffi::gboolean;
}

/// picker.js's texts (its own makeT table), Dutch and English.
fn tr(key: &str) -> &'static str {
    match key {
        "open" => t("Openen", "Open"),
        "save" => t("Opslaan", "Save"),
        "saveAs" => t("Opslaan als", "Save as"),
        "folder" => t("Map kiezen", "Choose a folder"),
        "cancel" => t("Annuleren", "Cancel"),
        "name" => t("Bestandsnaam", "File name"),
        "type" => t("Bestandstype", "File type"),
        "all" => t("Alle bestanden", "All files"),
        "required" => t("Geef een bestandsnaam op.", "Enter a file name."),
        "missing" => t("Dit bestand bestaat niet.", "This file doesn\u{2019}t exist."),
        "overwrite" => t("Bestand vervangen?", "Replace file?"),
        "replace" => t("Vervangen", "Replace"),
        "exists" => t(
            "\u{201c}{name}\u{201d} bestaat al. Wil je het vervangen?",
            "\u{201c}{name}\u{201d} already exists. Do you want to replace it?",
        ),
        "choose" => t("Kies een bestaande map.", "Choose an existing folder."),
        "readOnly" => t("Alleen-lezen openen", "Open read-only"),
        _ => "",
    }
}

// ============================================================================================ launch

/// What the caller asked for on stdin (`--picker-stdio`).
struct Request {
    options: Value,
    path: Option<String>,
    parent: Option<String>,
}

static REQUEST: OnceLock<Request> = OnceLock::new();
/// The caller talks to the picker over stdin and stdout.
static STDIO: AtomicBool = AtomicBool::new(false);

/// Called first in main: for a picker launch (`--picker-stdio`, or `--params` with "picker") read the request
/// (stdio) and return the picker's own GApplication id; None for a regular Explorer launch.
pub fn prepare(argv: &[String]) -> Result<Option<&'static str>, String> {
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(None);
    }
    if argv.iter().any(|a| a == "--picker-stdio") {
        let request = read_request()?;
        let _ = REQUEST.set(request);
        STDIO.store(true, Ordering::Relaxed);
    } else if crate::parse_args(argv).picker.is_none() {
        return Ok(None);
    }
    let id: &'static str = Box::leak(format!("{PICKER_APP_ID}.p{}", std::process::id()).into_boxed_str());
    Ok(Some(id))
}

fn read_request() -> Result<Request, String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .take(REQUEST_MAX)
        .read_line(&mut line)
        .map_err(|e| format!("picker request: {e}"))?;
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| format!("picker request: {e}"))?;
    let options = match &v["picker"] {
        Value::Object(_) => v["picker"].clone(),
        _ => return Err("picker request: no picker options".into()),
    };
    let path = v["path"].as_str().filter(|p| p.starts_with('/') && !p.contains('\0')).map(str::to_owned);
    let parent = v["parent"].as_str().filter(|p| !p.is_empty() && p.len() <= 1024).map(str::to_owned);
    Ok(Request { options, path, parent })
}

/// The launch's params with the stdin request in them (the picker options and the start folder).
pub fn launch_params(mut p: Params) -> Params {
    if let Some(r) = REQUEST.get() {
        p.picker = Some(r.options.clone());
        p.path = r.path.clone();
        p.place = None;
        p.tabs = None;
        p.select = None;
        p.properties = false;
        p.tab_ticket = None;
    }
    p
}

// ============================================================================================ filters

/// One part of a glob (picker.js matchPickerGlob turns the pattern into a RegExp; this is the same language).
#[derive(Debug, PartialEq)]
enum Tok {
    /// `*`: any run of characters (no line breaks, like RegExp's `.*`).
    Any,
    /// `?`: one character (no line break).
    One,
    Lit(char),
    /// `[...]` / `[!...]` / `[^...]`: ranges (a single character is a range of one).
    Class { negate: bool, ranges: Vec<(char, char)> },
}

fn line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// The class body as ranges, or None when RegExp would reject it (a range that runs backwards).
fn class_ranges(body: &[char]) -> Option<Vec<(char, char)>> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < body.len() {
        if k + 2 < body.len() && body[k + 1] == '-' {
            if body[k] > body[k + 2] {
                return None;
            }
            out.push((body[k], body[k + 2]));
            k += 3;
        } else {
            out.push((body[k], body[k]));
            k += 1;
        }
    }
    Some(out)
}

fn compile_glob(pattern: &str) -> Vec<Tok> {
    let p: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '*' => out.push(Tok::Any),
            '?' => out.push(Tok::One),
            '[' => {
                let mut end = i + 1;
                if end < p.len() && (p[end] == '!' || p[end] == '^') {
                    end += 1;
                }
                if end < p.len() && p[end] == ']' {
                    end += 1;
                }
                if let Some(close) = (end..p.len()).find(|&k| p[k] == ']')
                    && close > i + 1
                {
                    let mut body = &p[i + 1..close];
                    let negate = matches!(body.first(), Some('!' | '^'));
                    if negate {
                        body = &body[1..];
                    }
                    if !body.is_empty()
                        && let Some(ranges) = class_ranges(body)
                    {
                        out.push(Tok::Class { negate, ranges });
                        i = close + 1;
                        continue;
                    }
                }
                out.push(Tok::Lit('['));
            }
            c => out.push(Tok::Lit(c)),
        }
        i += 1;
    }
    out
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn tok_matches(tok: &Tok, c: char, case_sensitive: bool) -> bool {
    match tok {
        Tok::Any => true,
        Tok::One => !line_break(c),
        Tok::Lit(l) => *l == c || (!case_sensitive && fold(*l) == fold(c)),
        Tok::Class { negate, ranges } => {
            let within = |x: char| ranges.iter().any(|(a, b)| *a <= x && x <= *b);
            let hit = within(c)
                || (!case_sensitive && (within(fold(c)) || c.to_uppercase().next().is_some_and(within)));
            hit != *negate
        }
    }
}

/// Whether the whole `value` matches the glob (picker.js matchPickerGlob).
fn glob_matches(toks: &[Tok], value: &str, case_sensitive: bool) -> bool {
    let s: Vec<char> = value.chars().collect();
    let (mut ti, mut si) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if ti < toks.len() {
            if toks[ti] == Tok::Any {
                star = Some((ti, si));
                ti += 1;
                continue;
            }
            if tok_matches(&toks[ti], s[si], case_sensitive) {
                ti += 1;
                si += 1;
                continue;
            }
        }
        match star {
            Some((st, ss)) if !line_break(s[ss]) => {
                star = Some((st, ss + 1));
                ti = st + 1;
                si = ss + 1;
            }
            _ => return false,
        }
    }
    toks[ti..].iter().all(|t| *t == Tok::Any)
}

/// One rule of a filter.
enum Rule {
    /// "*", or "*.*" when not case sensitive.
    All,
    Glob(Vec<Tok>),
    Mime(String),
    /// "image/*": the MIME type starts with "image/".
    MimePrefix(String),
}

struct Filter {
    name: String,
    rules: Vec<Rule>,
    case_sensitive: bool,
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn parse_filter(f: &Value) -> Filter {
    let case_sensitive = f["caseSensitive"] == true;
    let mut conditions: Vec<(bool, String)> = Vec::new();
    if let Some(list) = f["conditions"].as_array() {
        for c in list.iter().filter(|c| c.is_object()) {
            conditions.push((c["type"].as_f64() == Some(1.0), text_of(&c["value"])));
        }
    } else {
        let patterns = f["patterns"].as_array().cloned().unwrap_or_else(|| vec![json!("*")]);
        for p in &patterns {
            let p = text_of(p);
            conditions.push((p.contains('/'), p));
        }
    }
    let rules = conditions
        .into_iter()
        .map(|(mime, pattern)| {
            if mime {
                if pattern.ends_with("/*") {
                    Rule::MimePrefix(pattern[..pattern.len() - 1].to_owned())
                } else {
                    Rule::Mime(pattern)
                }
            } else if pattern == "*" || (!case_sensitive && pattern == "*.*") {
                Rule::All
            } else {
                Rule::Glob(compile_glob(&pattern))
            }
        })
        .collect();
    let name = f["name"].as_str().filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or_else(|| tr("all").to_owned());
    Filter { name, rules, case_sensitive }
}

/// A drive tile (Deze pc) carries its volume.
fn has_volume(item: &Value) -> bool {
    matches!(item.get("volume"), Some(v) if v.is_object())
}

/// The path a chosen item stands for (a drive: its mount point).
fn item_path(item: &Value) -> String {
    let mount = item.get("volume").map(|v| st(v, "mount")).unwrap_or("");
    if !mount.is_empty() { mount.to_owned() } else { st(item, "path").to_owned() }
}

fn item_name(item: &Value) -> String {
    let n = st(item, "name");
    if n.is_empty() { util::basename(st(item, "path")) } else { n.to_owned() }
}

// ============================================================================================ the footer

struct State {
    x: WeakExplorer,
    options: Value,
    folder: bool,
    save: bool,
    multiple: bool,
    stdio: bool,
    filters: Vec<Filter>,
    filter: Cell<usize>,
    /// A submit is being checked (files.stat, the overwrite question) or waits for the caller's answer.
    busy: Cell<bool>,
    /// The caller has the submit and decides.
    waiting: Cell<bool>,
    /// The user typed in the name box after the last selection change.
    typed: Cell<bool>,
    /// The name box is being set by the picker (its changed signal is no typing).
    setting: Cell<bool>,
    /// An answer (submit or close) went out: the window may close without another one.
    answered: Cell<bool>,
    choices: RefCell<Vec<(String, String)>>,
    writable: Cell<bool>,
    name: gtk::Entry,
    name_label: gtk::Label,
    type_label: gtk::Label,
    name_row: gtk::Box,
    type_row: gtk::Box,
    error: gtk::Label,
    choose: gtk::Button,
    cancel: gtk::Button,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
    static CSS_DONE: Cell<bool> = const { Cell::new(false) };
}

fn install_css() {
    if CSS_DONE.with(|d| d.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let p = gtk::CssProvider::new();
    p.connect_parsing_error(|_, section, error| {
        eprintln!("win11-gtk-explorer: picker css {}: {}", section.to_str(), error);
    });
    p.load_from_string(CSS);
    // above the kit's sheet and the Explorer's (install_app_css is APPLICATION + 2)
    gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
}

/// A row of the footer: a 110 px caption and its control (`.xp-picker__label`).
fn field_row(caption: &gtk::Label, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("xp-picker-label");
    caption.set_size_request(110, -1);
    caption.set_xalign(0.0);
    caption.set_valign(gtk::Align::Center);
    row.append(caption);
    control.set_valign(gtk::Align::Center);
    row.append(control);
    row
}

/// Turn the window into the picker for these options.
pub fn attach(x: &Explorer, options: &Value) {
    install_css();
    let o = options;
    let folder = o["folder"] == true;
    let save = !folder && (o["save"] == true || o["action"] == "save");
    let multiple = o["multiple"] == true && !save;
    let mut filters: Vec<Filter> =
        o["filters"].as_array().into_iter().flatten().filter(|f| f.is_object()).take(32).map(parse_filter).collect();
    if filters.is_empty() {
        filters.push(Filter { name: tr("all").to_owned(), rules: vec![Rule::All], case_sensitive: false });
    }
    // Number(options.currentFilter) || 0
    let current = match &o["currentFilter"] {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    let current = current.filter(|n| n.is_finite()).unwrap_or(0.0).trunc().max(0.0) as usize;
    let filter = current.min(filters.len() - 1);

    // file name
    let name = controls::text_box("");
    name.add_css_class("xp-picker-name");
    name.set_hexpand(true);
    name.set_widget_name("explorer-picker-name");
    let initial = o["name"].as_str().filter(|s| !s.is_empty()).or_else(|| o["fileName"].as_str()).unwrap_or("");
    name.set_text(initial);
    let name_label = controls::label(tr("name"), "", false);
    let name_row = field_row(&name_label, &name);
    // file type
    let types = Dropdown::new(
        filters.iter().enumerate().map(|(i, f)| DropdownOption::new(&i.to_string(), &f.name)).collect(),
        Some(&filter.to_string()),
    );
    types.widget().set_size_request(300, -1);
    types.widget().set_widget_name("explorer-picker-type");
    let type_label = controls::label(tr("type"), "", false);
    let type_row = field_row(&type_label, types.widget());
    // buttons and the error line
    let choose = controls::button("", Variant::Accent);
    choose.set_widget_name("explorer-picker-accept");
    let cancel = controls::button(tr("cancel"), Variant::Standard);
    cancel.set_widget_name("explorer-picker-cancel");
    let error = controls::label("", "xp-picker-error", true);
    error.set_visible(false);
    error.set_widget_name("explorer-picker-error");

    let state = Rc::new(State {
        x: x.weak(),
        options: o.clone(),
        folder,
        save,
        multiple,
        stdio: STDIO.load(Ordering::Relaxed),
        filters,
        filter: Cell::new(filter),
        busy: Cell::new(false),
        waiting: Cell::new(false),
        typed: Cell::new(false),
        setting: Cell::new(false),
        answered: Cell::new(false),
        choices: RefCell::new(Vec::new()),
        writable: Cell::new(o["writable"] != false),
        name,
        name_label,
        type_label,
        name_row,
        type_row,
        error,
        choose,
        cancel,
    });
    CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));

    // the portal's choices: a check box (no options) or a caption with a drop-down
    let mut controls_list: Vec<gtk::Widget> = Vec::new();
    for choice in o["choices"].as_array().into_iter().flatten().take(32) {
        let Some(id) = choice["id"].as_str() else { continue };
        let id = id.to_owned();
        let caption = choice["label"].as_str().filter(|l| !l.is_empty()).unwrap_or(&id).to_owned();
        let items: Vec<Value> = choice["options"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            let on = choice["selected"] == "true";
            state.set_choice(&id, if on { "true" } else { "false" });
            let c = controls::checkbox(&caption);
            c.set_active(on);
            let s = Rc::downgrade(&state);
            c.connect_toggled(move |c| {
                if let Some(s) = s.upgrade() {
                    s.set_choice(&id, if c.is_active() { "true" } else { "false" });
                }
            });
            controls_list.push(c.upcast());
        } else {
            let selected = match &choice["selected"] {
                Value::Null => text_of(&items[0]["id"]),
                v => text_of(v),
            };
            state.set_choice(&id, &selected);
            let dd = Dropdown::new(
                items.iter().map(|i| DropdownOption::new(&text_of(&i["id"]), &text_of(&i["label"]))).collect(),
                Some(&selected),
            );
            dd.widget().set_size_request(300, -1);
            let s = Rc::downgrade(&state);
            dd.connect_changed(move |v| {
                if let Some(s) = s.upgrade() {
                    s.set_choice(&id, v);
                }
            });
            let cap = controls::label(&caption, "", false);
            controls_list.push(field_row(&cap, dd.widget()).upcast());
        }
    }
    let read_only = (o["allowReadOnly"] == true).then(|| {
        let c = controls::checkbox(tr("readOnly"));
        c.set_active(!state.writable.get());
        let s = Rc::downgrade(&state);
        c.connect_toggled(move |c| {
            if let Some(s) = s.upgrade() {
                s.writable.set(!c.is_active());
            }
        });
        c
    });

    // the footer (`.xp-picker__footer`): the fields (scrolling past 38 % of the window) and the buttons
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 10);
    fields.add_css_class("xp-picker-fields");
    fields.append(&state.name_row);
    fields.append(&state.type_row);
    for c in &controls_list {
        fields.append(c);
    }
    if let Some(c) = &read_only {
        fields.append(c);
    }
    fields.append(&state.error);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_propagate_natural_height(true);
    scroller.set_max_content_height(258);
    scroller.set_hexpand(true);
    scroller.set_child(Some(&fields));
    let buttons = gtk::Box::new(gtk::Orientation::Vertical, 10);
    buttons.add_css_class("xp-picker-buttons");
    buttons.set_size_request(100, -1);
    buttons.set_valign(gtk::Align::Start);
    buttons.append(&state.choose);
    buttons.append(&state.cancel);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    footer.add_css_class("xp-picker-footer");
    footer.set_widget_name("explorer-picker");
    footer.append(&scroller);
    footer.append(&buttons);
    state.name_row.set_visible(!folder);
    state.type_row.set_visible(!folder);

    // signals
    let s = Rc::downgrade(&state);
    state.name.connect_changed(move |_| {
        if let Some(s) = s.upgrade()
            && !s.setting.get()
        {
            s.typed.set(true);
        }
    });
    let s = Rc::downgrade(&state);
    state.name.connect_activate(move |_| {
        if let Some(s) = s.upgrade() {
            s.submit(None);
        }
    });
    let s = Rc::downgrade(&state);
    types.connect_changed(move |v| {
        if let Some(s) = s.upgrade() {
            s.filter.set(v.parse().unwrap_or(0).min(s.filters.len() - 1));
            if let Some(x) = s.x.upgrade() {
                x.refresh();
            }
        }
    });
    let s = Rc::downgrade(&state);
    state.choose.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.submit(None);
        }
    });
    let s = Rc::downgrade(&state);
    state.cancel.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.cancel_picker();
        }
    });

    // the window: its hooks, the footer, the title bar with the title instead of the tabs
    let s1 = Rc::downgrade(&state);
    let s2 = Rc::downgrade(&state);
    let s3 = Rc::downgrade(&state);
    let s4 = Rc::downgrade(&state);
    x.set_picker(Picker {
        filter: Rc::new(move |item: &Value| s1.upgrade().is_some_and(|s| s.matches(item))),
        on_selection: Rc::new(move |items: &[Item]| {
            if let Some(s) = s2.upgrade() {
                s.selection_changed(items);
            }
        }),
        on_select: Rc::new(move |item: &Item| {
            if let Some(s) = s3.upgrade() {
                s.set_name(&item_name(item));
                s.submit(Some(vec![st(item, "path").to_owned()]));
            }
        }),
        title: Rc::new(move || s4.upgrade().map(|s| s.title()).unwrap_or_else(|| tr("open").to_owned())),
    });
    x.set_footer(Some(footer.upcast_ref()));
    let win = x.window();
    win.set_titlebar_content(None::<&gtk::Widget>);
    win.set_title_widget(None::<&gtk::Widget>);
    win.window().add_css_class("xp-picker-window");
    kit::wayland::set_app_id(win.window(), PICKER_APP_ID);
    state.relabel();
    win.set_title(&state.title());
    x.update_chrome();
    if let Some(parent) = REQUEST.get().and_then(|r| r.parent.clone()) {
        set_parent(win.window(), &parent, o["modal"] != false);
    }

    // Escape cancels (before the Explorer's own keys; not while a dialog is open or a name is being edited)
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = Rc::downgrade(&state);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(s) = s.upgrade() else { return glib::Propagation::Proceed };
        if key != gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        let Some(x) = s.x.upgrade() else { return glib::Propagation::Proceed };
        if x.window().dialog_open() || x.view().is_some_and(|v| v.renaming()) {
            return glib::Propagation::Proceed;
        }
        s.cancel_picker();
        glib::Propagation::Stop
    });
    win.window().add_controller(keys);
    // closing the window any other way (caption button, Alt+F4) cancels too
    let s = Rc::downgrade(&state);
    win.window().connect_close_request(move |_| {
        if let Some(s) = s.upgrade()
            && !s.answered.replace(true)
        {
            post(&json!({ "cmd": "close" }));
        }
        glib::Propagation::Proceed
    });
    // the caller's answers
    if state.stdio {
        read_answers();
    }
    let s = Rc::downgrade(&state);
    kit::i18n::on_change(move || {
        if let Some(s) = s.upgrade() {
            s.relabel();
            if let Some(x) = s.x.upgrade() {
                x.update_chrome();
            }
        }
    });
}

/// One JSON line to the caller (stdout).
fn post(msg: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

/// Read the caller's lines (stdin) on a thread; each is handled on the main thread, the end of stdin too.
fn read_answers() {
    let spawned = std::thread::Builder::new().name("win11-picker-stdin".into()).spawn(|| {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        loop {
            let mut line = String::new();
            let read = lock.by_ref().take(1 << 20).read_line(&mut line);
            let done = !matches!(read, Ok(n) if n > 0);
            let msg = (!done).then_some(line);
            glib::MainContext::default().invoke(move || answer(msg));
            if done {
                break;
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("win11-gtk-explorer: picker stdin: {e}");
    }
}

/// The caller's answer to a submit: `{"ok": true}` closes, `{"error"}` shows it; None: the caller is gone.
fn answer(line: Option<String>) {
    let Some(s) = CURRENT.with(|c| c.borrow().clone()) else { return };
    let Some(line) = line else {
        s.answered.set(true);
        s.close_window();
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return };
    if v["ok"] == true {
        s.answered.set(true);
        s.close_window();
    } else if let Some(e) = v["error"].as_str() {
        s.waiting.set(false);
        s.set_busy(false);
        s.set_error(e);
    }
}

/// Make the picker transient for the caller's exported Wayland surface (xdg-foreign) and modal.
fn set_parent(window: &gtk::ApplicationWindow, handle: &str, modal: bool) {
    let Some(raw) = handle.strip_prefix("wayland:").filter(|h| !h.is_empty() && h.len() <= 1024) else {
        if !handle.is_empty() {
            eprintln!("win11-gtk-explorer: picker parent {handle:?} is not a Wayland handle; opening without a parent");
        }
        return;
    };
    let Ok(raw) = std::ffi::CString::new(raw) else { return };
    let apply = move |w: &gtk::Window| {
        let Some(surface) = w.surface() else { return };
        if !surface.display().type_().name().contains("Wayland") {
            return;
        }
        let Ok(top) = surface.downcast::<gdk::Toplevel>() else { return };
        let ok = unsafe { gdk_wayland_toplevel_set_transient_for_exported(top.to_glib_none().0, raw.as_ptr()) };
        if ok == glib::ffi::GFALSE {
            eprintln!("win11-gtk-explorer: the picker's parent could not be imported; opening without a parent");
        }
    };
    window.set_modal(modal);
    let w: &gtk::Window = window.upcast_ref();
    if w.is_realized() {
        apply(w);
    }
    let again = apply.clone();
    w.connect_realize(move |w| again(w));
}

/// Why a submit stopped without an answer.
enum Stop {
    /// A message for the error line.
    Error(String),
    /// Nothing to say (a folder was opened, the overwrite question was declined).
    Quiet,
}

impl State {
    fn set_choice(&self, id: &str, value: &str) {
        let mut c = self.choices.borrow_mut();
        match c.iter_mut().find(|(k, _)| k == id) {
            Some(e) => e.1 = value.to_owned(),
            None => c.push((id.to_owned(), value.to_owned())),
        }
    }

    fn title(&self) -> String {
        if let Some(own) = self.options["title"].as_str().filter(|s| !s.is_empty()) {
            return own.to_owned();
        }
        tr(if self.folder { "folder" } else if self.save { "saveAs" } else { "open" }).to_owned()
    }

    fn button_label(&self) -> String {
        if let Some(own) = self.options["acceptLabel"].as_str().filter(|s| !s.is_empty()) {
            return own.to_owned();
        }
        tr(if self.folder { "folder" } else if self.save { "save" } else { "open" }).to_owned()
    }

    fn relabel(&self) {
        self.name_label.set_label(tr("name"));
        self.type_label.set_label(tr("type"));
        self.choose.set_label(&self.button_label());
        self.cancel.set_label(tr("cancel"));
    }

    fn set_name(&self, text: &str) {
        self.setting.set(true);
        self.name.set_text(text);
        self.name.set_position(-1);
        self.setting.set(false);
    }

    fn set_error(&self, text: &str) {
        self.error.set_label(text);
        self.error.set_visible(!text.is_empty());
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.choose.set_sensitive(!busy);
    }

    /// picker.js matches: folders and drives always; in a folder picker nothing else; else the chosen filter.
    fn matches(&self, item: &Value) -> bool {
        if flag(item, "isDir") || has_volume(item) {
            return true;
        }
        if self.folder {
            return false;
        }
        let Some(f) = self.filters.get(self.filter.get()) else { return true };
        let name = st(item, "name");
        let mime = st(item, "mime");
        f.rules.iter().any(|r| match r {
            Rule::All => true,
            Rule::Glob(toks) => glob_matches(toks, name, f.case_sensitive),
            Rule::Mime(m) => mime == m,
            Rule::MimePrefix(p) => mime.starts_with(p.as_str()),
        })
    }

    /// picker.js pickerSelection: one file puts its name in the box, several (multiple) their quoted names.
    fn selection_changed(&self, items: &[Item]) {
        self.typed.set(false);
        if items.len() == 1 && !flag(&items[0], "isDir") && !has_volume(&items[0]) {
            self.set_name(&item_name(&items[0]));
        } else if self.multiple && items.len() > 1 {
            let names: Vec<String> = items.iter().map(|i| format!("\u{201c}{}\u{201d}", item_name(i))).collect();
            self.set_name(&names.join(" "));
        }
    }

    /// Annuleren, Escape: the caller hears "close" and the window goes.
    fn cancel_picker(&self) {
        if !self.answered.replace(true) {
            post(&json!({ "cmd": "close" }));
        }
        self.close_window();
    }

    fn close_window(&self) {
        if let Some(x) = self.x.upgrade() {
            x.window().close();
        }
    }

    /// picker.js submit: work out the paths, check them (files.stat), ask before replacing, then answer.
    fn submit(self: &Rc<Self>, explicit: Option<Vec<String>>) {
        if self.busy.get() || self.waiting.get() || self.answered.get() {
            return;
        }
        self.set_error("");
        self.set_busy(true);
        let me = self.clone();
        glib::spawn_future_local(async move {
            match me.check(explicit).await {
                Ok(paths) => me.send(paths),
                Err(Stop::Error(msg)) => {
                    me.set_error(&msg);
                    me.set_busy(false);
                }
                Err(Stop::Quiet) => me.set_busy(false),
            }
        });
    }

    async fn check(&self, explicit: Option<Vec<String>>) -> Result<Vec<String>, Stop> {
        let Some(x) = self.x.upgrade() else { return Err(Stop::Quiet) };
        let ctx = x.ctx();
        let loc = x.loc();
        let mut paths = explicit;
        if paths.is_none() && self.multiple && !self.typed.get() {
            let selected: Vec<String> = x
                .selected_items()
                .iter()
                .filter(|i| {
                    let dir = flag(i, "isDir") || has_volume(i);
                    if self.folder { dir } else { !dir }
                })
                .map(|i| item_path(i))
                .filter(|p| !p.is_empty())
                .collect();
            if !selected.is_empty() {
                paths = Some(selected);
            }
        }
        if paths.is_none() && self.folder {
            let chosen = x.selected_items().into_iter().find(|i| flag(i, "isDir") || has_volume(i)).map(|i| item_path(&i));
            let path = chosen.filter(|p| !p.is_empty()).or_else(|| match &loc {
                Loc::Folder(p) => Some(p.clone()),
                _ => None,
            });
            if let Some(p) = path {
                paths = Some(vec![p]);
            }
        }
        if paths.is_none() && !self.folder {
            let value = self.name.text().to_string();
            if value.is_empty() || value.contains('\0') {
                return Err(Stop::Error(tr("required").into()));
            }
            let path = if value.starts_with('/') {
                Some(util::norm_path(&value))
            } else if let Loc::Folder(dir) = &loc {
                Some(util::join_path(dir, &value))
            } else {
                None
            };
            if let Some(p) = path {
                paths = Some(vec![p]);
            }
        }
        let mut paths = paths.filter(|p| !p.is_empty()).ok_or_else(|| Stop::Error(tr("choose").into()))?;
        if !self.multiple && paths.len() != 1 {
            paths.truncate(1);
        }
        for path in &paths {
            let stat = match ctx.call_async("files.stat", json!({ "path": path })).await {
                Ok(v) => Some(v),
                Err(e) if e.code == "not_found" => None,
                Err(e) => return Err(Stop::Error(e.message)),
            };
            let is_dir = stat.as_ref().is_some_and(|s| flag(s, "isDir"));
            if self.folder {
                if !is_dir {
                    return Err(Stop::Error(tr("choose").into()));
                }
            } else if is_dir {
                // a folder in Open or Save mode: go there
                x.navigate(Loc::Folder(path.clone()), NavOpts { focus: true, ..Default::default() });
                self.set_name("");
                self.typed.set(false);
                return Err(Stop::Quiet);
            } else if !self.save && stat.is_none() {
                return Err(Stop::Error(tr("missing").into()));
            }
            if self.save {
                let parent = ctx
                    .call_async("files.stat", json!({ "path": util::dirname(path) }))
                    .await
                    .map_err(|e| Stop::Error(e.message))?;
                if !flag(&parent, "isDir") {
                    return Err(Stop::Error(tr("choose").into()));
                }
                if stat.is_some() && !self.confirm_replace(&x, path).await {
                    return Err(Stop::Quiet);
                }
            }
        }
        Ok(paths)
    }

    /// Dialog.confirm "Bestand vervangen?" with Vervangen / Annuleren.
    async fn confirm_replace(&self, x: &Explorer, path: &str) -> bool {
        let (tx, rx) = oneshot();
        let message = tr("exists").replace("{name}", &util::basename(path));
        controls::confirm(&x.anchor(), tr("overwrite"), &message, tr("replace"), false, move |ok| tx.send(ok));
        rx.recv().await
    }

    /// The answer: picker-submit to the caller; stdio waits for its verdict, else the window closes.
    fn send(&self, paths: Vec<String>) {
        let choices: Vec<Value> = self.choices.borrow().iter().map(|(k, v)| json!([k, v])).collect();
        let msg = json!({
            "cmd": "picker-submit",
            "path": paths[0],
            "paths": paths,
            "filterIndex": self.filter.get(),
            "choices": choices,
            "writable": self.writable.get(),
        });
        post(&msg);
        if self.stdio {
            self.waiting.set(true);
        } else {
            self.answered.set(true);
            self.close_window();
        }
    }
}

/// A one-shot answer from a callback to an async fn on the main thread (no extra crates).
struct OneshotTx(Rc<RefCell<(Option<bool>, Option<std::task::Waker>)>>);
struct OneshotRx(Rc<RefCell<(Option<bool>, Option<std::task::Waker>)>>);

fn oneshot() -> (OneshotTx, OneshotRx) {
    let cell = Rc::new(RefCell::new((None, None)));
    (OneshotTx(cell.clone()), OneshotRx(cell))
}

impl OneshotTx {
    fn send(self, v: bool) {
        let waker = {
            let mut c = self.0.borrow_mut();
            c.0 = Some(v);
            c.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl OneshotRx {
    async fn recv(self) -> bool {
        std::future::poll_fn(move |cx| {
            let mut c = self.0.borrow_mut();
            match c.0 {
                Some(v) => std::task::Poll::Ready(v),
                None => {
                    c.1 = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(p: &str, v: &str, cs: bool) -> bool {
        glob_matches(&compile_glob(p), v, cs)
    }

    #[test]
    fn globs_follow_match_picker_glob() {
        assert!(g("*.txt", "a.txt", true));
        assert!(!g("*.txt", "a.TXT", true));
        assert!(g("*.txt", "a.TXT", false));
        assert!(g("?.png", "a.png", true));
        assert!(!g("?.png", "ab.png", true));
        assert!(g("*.[jJ][pP][gG]", "photo.JPG", true));
        assert!(!g("*.[jJ][pP]g", "photo.JPG", true));
        assert!(g("[!a]*", "b", true));
        assert!(!g("[!a]*", "abc", true));
        assert!(g("[^a]x", "bx", true));
        assert!(g("[a-c]", "b", true));
        assert!(!g("[a-c]", "d", true));
        // a class that runs backwards is a literal "["
        assert!(g("[c-a]", "[c-a]", true));
        // no closing bracket: literal
        assert!(g("[ab", "[ab", true));
        assert!(g("[]]", "]", true));
        assert!(g("a*b*c", "aXXbYYc", true));
        assert!(!g("a*b*c", "aXXbYY", true));
        assert!(g("*", "", true));
        assert!(!g("*", "a\nb", true));
        assert!(g("a.b", "a.b", true));
        assert!(!g("a.b", "aXb", true));
    }

    #[test]
    fn filters_take_patterns_conditions_and_mime_rules() {
        let f = parse_filter(&json!({"name": "Images", "patterns": ["*.png", "image/*"]}));
        assert_eq!(f.rules.len(), 2);
        assert!(matches!(f.rules[1], Rule::MimePrefix(ref p) if p == "image/"));
        let f = parse_filter(&json!({"conditions": [{"type": 1, "value": "text/plain"}, {"type": 0, "value": "*.*"}], "caseSensitive": true}));
        assert!(matches!(f.rules[0], Rule::Mime(ref m) if m == "text/plain"));
        assert!(matches!(f.rules[1], Rule::Glob(_)));
        let f = parse_filter(&json!({"patterns": ["*.*"]}));
        assert!(matches!(f.rules[0], Rule::All));
    }
}
