//! The Alles window (main.js): the big search box in the title bar, the command bar (kind, Hoofdletters /
//! Heel woord / Pad / Patroon, Sorteren, Zoektips, Voorbeeld, Opties), the result table with its overlays
//! (welcome, first run, nothing found, errors), the panes and the status bar with the index state.
use crate::actions;
use crate::art;
use crate::highlight::Highlight;
use crate::model::{self, Entry, PAGE, Results};
use crate::options;
use crate::panes::{self, PreviewPane};
use crate::table::{self, ResultsTable};
use crate::text::{self, FILTERS, SORTS};
use kit::controls::{self, Dropdown, DropdownOption, InfoBar, MenuItem, ProgressBar, ProgressRing, Severity, TopAction};
use kit::frame::{AppWindow, WindowOptions};
use kit::i18n;
use kit::prelude::*;
use kit::{Ctx, t};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// At most this many paths are copied at once (main.js COPY_MAX).
const COPY_MAX: usize = 10000;

/// What a launch asks for (main.js params: query / q / search / text, path, filter, pane; select, open and
/// scrollTo are main.js's screenshot and test helpers).
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub query: Option<String>,
    pub path: Option<String>,
    pub filter: Option<String>,
    pub pane: Option<String>,
    pub select: Option<String>,
    pub open: Option<String>,
    pub scroll_to: Option<String>,
}

impl Params {
    /// From the arguments of a launch: `--params JSON` (the host's shell.openApp) or a path.
    pub fn parse(args: &[String]) -> Params {
        let mut p = Params::default();
        let args: Vec<&String> = args.iter().filter(|a| !matches!(a.as_str(), "--dry-run" | "--http-only")).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--params" => {
                    if let Some(raw) = args.get(i + 1).filter(|r| r.len() <= 64 * 1024)
                        && let Ok(v) = serde_json::from_str::<Value>(raw)
                        && v.is_object()
                    {
                        p.merge(&v);
                    }
                    i += 2;
                }
                "--tab-ticket" | "--app" => i += 2,
                a if a.starts_with("--") => i += 1,
                a => {
                    if p.path.is_none() && !a.is_empty() {
                        p.path = Some(a.to_owned());
                    }
                    i += 1;
                }
            }
        }
        p
    }

    fn merge(&mut self, v: &Value) {
        let s = |k: &str| match &v[k] {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        };
        // withQuery: q / search / text from other callers
        self.query = v["query"].as_str().map(str::to_owned).or_else(|| ["q", "search", "text"].iter().find_map(|k| v[*k].as_str().map(str::to_owned)));
        self.path = s("path").filter(|p| !p.is_empty()).or(self.path.take());
        self.filter = s("filter");
        self.pane = s("pane");
        self.select = s("select");
        self.open = s("open");
        self.scroll_to = s("scrollTo");
    }
}

// ------------------------------------------------------------------------------------------ remembered view

/// The view kept between runs (main.js localStorage win11shell.alles.view and .columns):
/// $XDG_CONFIG_HOME/win11-gnome/native-alles.json {sort, desc, pane, widths}.
fn view_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(glib::user_config_dir);
    base.join("win11-gnome").join("native-alles.json")
}

fn load_view() -> Value {
    std::fs::read(view_path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn save_view(patch: Value) {
    let mut v = load_view();
    if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in p {
            obj.insert(k.clone(), val.clone());
        }
    }
    let path = view_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// "Bijgewerkt 5 minuten geleden" (st_builtAt).
pub fn built_at(ms: f64) -> String {
    let when = kit::format::rel_time(ms, kit::format::now_ms());
    if i18n::en() { format!("Updated {when}") } else { format!("Bijgewerkt {when}") }
}

// ---------------------------------------------------------------------------------------------- the state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opt {
    MatchCase,
    WholeWord,
    MatchPath,
    Regex,
}

impl Opt {
    const ALL: [Opt; 4] = [Opt::MatchCase, Opt::WholeWord, Opt::MatchPath, Opt::Regex];

    fn key(self) -> &'static str {
        match self {
            Opt::MatchCase => "matchCase",
            Opt::WholeWord => "wholeWord",
            Opt::MatchPath => "matchPath",
            Opt::Regex => "regex",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Opt::MatchCase => t("Hoofdletters", "Match case"),
            Opt::WholeWord => t("Heel woord", "Whole word"),
            Opt::MatchPath => t("Pad", "Path"),
            Opt::Regex => t("Patroon", "Pattern"),
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Opt::MatchCase => t("Hoofdlettergevoelig zoeken (Alt+C)", "Match case (Alt+C)"),
            Opt::WholeWord => t("Alleen hele woorden vinden (Alt+W)", "Match whole words only (Alt+W)"),
            Opt::MatchPath => t("Ook in mapnamen en het volledige pad zoeken (Alt+P)", "Also search folder names and the full path (Alt+P)"),
            Opt::Regex => t(
                "Zoeken met een patroon (reguliere expressie), voor gevorderden (Alt+R)",
                "Search with a pattern (regular expression), for advanced users (Alt+R)",
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Overlay {
    Welcome,
    First,
    Empty(String),
    QueryError(String),
    Unavailable(String),
}

impl Overlay {
    fn same_kind(&self, o: &Overlay) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o)
    }
}

#[derive(Default)]
struct State {
    query: String,
    filter: String,
    opts: [bool; 4],
    sort: String,
    desc: bool,
    pane: Option<&'static str>,
    status: Option<Value>,
    /// the last answer: (total, tookMs)
    result: Option<(u32, Option<f64>)>,
    seq: u64,
    args: Option<Value>,
    overlay: Option<Overlay>,
    index_kind: String,
    query_pending: bool,
    pending_enter: Option<bool>,
    focus: Option<u32>,
    narrow: u8,
}

type Update = Rc<dyn Fn(&Rc<App>)>;

struct Toggle {
    opt: Opt,
    button: gtk::ToggleButton,
    label: gtk::Label,
}

struct CmdButton {
    button: gtk::Button,
    label: gtk::Label,
}

pub struct App {
    ctx: Rc<Ctx>,
    win: AppWindow,
    search: gtk::Text,
    clear: gtk::Button,
    filter: Dropdown,
    toggles: Vec<Toggle>,
    sort_btn: CmdButton,
    tips_btn: gtk::ToggleButton,
    tips_label: gtk::Label,
    preview_btn: gtk::ToggleButton,
    preview_label: gtk::Label,
    options_btn: CmdButton,
    model: Results,
    table: ResultsTable,
    results: gtk::Overlay,
    overlay_host: gtk::ScrolledWindow,
    overlay_box: gtk::Box,
    pane_host: gtk::Box,
    preview: PreviewPane,
    count: gtk::Label,
    sel: gtk::Label,
    sel_sep: gtk::Widget,
    ms: gtk::Label,
    ms_sep: gtk::Widget,
    idx: gtk::Box,
    st: RefCell<State>,
    update_overlay: RefCell<Option<Update>>,
    update_index: RefCell<Option<Update>>,
    query_timer: RefCell<Option<glib::SourceId>>,
    preview_timer: RefCell<Option<glib::SourceId>>,
    changed_timer: RefCell<Option<glib::SourceId>>,
    changed_last: Cell<Option<Instant>>,
    width_timer: RefCell<Option<glib::SourceId>>,
    silent: Cell<bool>,
    syncing: Cell<bool>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn sep() -> gtk::Widget {
    let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    s.add_css_class("alles-cmd-sep");
    s.set_valign(gtk::Align::Center);
    s.upcast()
}

fn cmd_button(glyph: &str, chevron: bool) -> CmdButton {
    let button = gtk::Button::new();
    for c in ["w11-button", "w11-subtle", "alles-cmdbtn"] {
        button.add_css_class(c);
    }
    button.set_valign(gtk::Align::Center);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&kit::icons::glyph(glyph, 16));
    let label = gtk::Label::new(None);
    label.add_css_class("alles-cmdbtn-label");
    row.append(&label);
    if chevron {
        let c = kit::icons::glyph("chevron-down", 12);
        c.add_css_class("alles-cmdbtn-chev");
        row.append(&c);
    }
    button.set_child(Some(&row));
    CmdButton { button, label }
}

fn pane_button(glyph: &str) -> (gtk::ToggleButton, gtk::Label) {
    let button = gtk::ToggleButton::new();
    for c in ["w11-button", "w11-subtle", "alles-cmdbtn"] {
        button.add_css_class(c);
    }
    button.set_valign(gtk::Align::Center);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&kit::icons::glyph(glyph, 16));
    let label = gtk::Label::new(None);
    label.add_css_class("alles-cmdbtn-label");
    row.append(&label);
    button.set_child(Some(&row));
    (button, label)
}

fn filter_options() -> Vec<DropdownOption> {
    let zip = art::zip_icon_path();
    FILTERS
        .iter()
        .map(|f| {
            let o = DropdownOption::new(f, text::filter_label(f));
            match *f {
                "all" => o.glyph("grid"),
                "audio" => o.glyph("music"),
                "compressed" => match &zip {
                    Some(p) => o.app_icon(p),
                    None => o.glyph("file"),
                },
                "document" => o.glyph("document"),
                "executable" => o.glyph("app"),
                "folder" => o.glyph("folder"),
                "picture" => o.glyph("image"),
                _ => o.glyph("video"),
            }
        })
        .collect()
}

/// `(^|\s|!)(path|pad):` in the query (main.js: the path column is highlighted too).
fn has_path_modifier(q: &str) -> bool {
    let lower = q.to_lowercase();
    for word in ["path:", "pad:"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(word) {
            let at = from + i;
            let before = lower[..at].chars().last();
            if before.is_none_or(|c| c.is_whitespace() || c == '!') {
                return true;
            }
            from = at + word.len();
        }
    }
    false
}

/// Live updates are on and the home folder is watched (main.js `st.liveUpdate && st.watching`).
fn live_on(st: &Value) -> bool {
    st["liveUpdate"] == true && (st["watching"].as_f64().is_some_and(|w| w != 0.0) || st["watching"] == true)
}

/// Still typing a modifier: the last error is not shown yet (main.js `typing`).
fn still_typing(q: &str) -> bool {
    let last = q.chars().last();
    if last.is_some_and(|c| ":<>=.;,|!\"".contains(c)) {
        return true;
    }
    q.strip_suffix("..").and_then(|r| r.chars().last()).is_some_and(|c| ":<>=".contains(c))
}

impl App {
    /// The first launch: build the window and start.
    pub fn start(ctx: &Rc<Ctx>, params: Params) {
        kit::theme::install_app_css(include_str!("../css/alles.css"));
        let view = load_view();
        let widths: HashMap<String, i32> = view["widths"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()? as i32))).collect())
            .unwrap_or_default();
        let win = AppWindow::new(WindowOptions {
            key: "main".into(),
            title: String::new(),
            icon: "-".into(),
            width: 1040,
            height: 680,
            min_width: 640,
            min_height: 480,
            titlebar_height: 48,
            back: false,
        });
        win.titlebar().add_css_class("alles-titlebar");

        // ---- title bar: the logo, "Alles" and the big search box
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let logo_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        logo_slot.add_css_class("w11-appicon");
        logo_slot.set_valign(gtk::Align::Center);
        logo_slot.append(&art::logo(16));
        let title_label = gtk::Label::new(Some("Alles"));
        title_label.add_css_class("w11-apptitle");
        title_label.set_valign(gtk::Align::Center);
        title.append(&logo_slot);
        title.append(&title_label);
        win.set_title_widget(Some(&title));

        let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        search_box.add_css_class("w11-searchbox");
        search_box.add_css_class("alles-search");
        search_box.set_valign(gtk::Align::Center);
        search_box.set_hexpand(true);
        let lead = kit::icons::glyph("search", 16);
        lead.add_css_class("alles-search-icon");
        lead.set_valign(gtk::Align::Center);
        search_box.append(&lead);
        let search = gtk::Text::new();
        search.set_hexpand(true);
        search.set_valign(gtk::Align::Center);
        search.set_widget_name("alles-search");
        search.set_input_hints(gtk::InputHints::NO_SPELLCHECK);
        i18n::bind(&search, |s| s.set_placeholder_text(Some(t("Zoek in al je bestanden en mappen", "Search all your files and folders"))));
        search_box.append(&search);
        let clear = gtk::Button::new();
        clear.add_css_class("w11-searchbox-button");
        clear.set_child(Some(&kit::icons::glyph("close", 12)));
        clear.set_can_focus(false);
        clear.set_valign(gtk::Align::Center);
        clear.set_visible(false);
        i18n::tooltip(&clear, "Wissen", "Clear");
        search_box.append(&clear);
        let clamp = crate::clamp::Clamp::new(&search_box, 680);
        win.set_titlebar_content(Some(&clamp));

        // ---- command bar
        let cmd = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        cmd.add_css_class("alles-cmd");
        let filter = Dropdown::new(filter_options(), Some("all"));
        filter.widget().add_css_class("alles-filter");
        filter.widget().set_widget_name("alles-filter");
        filter.widget().set_size_request(168, -1);
        filter.widget().set_valign(gtk::Align::Center);
        cmd.append(filter.widget());
        cmd.append(&sep());
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let mut toggles = Vec::new();
        for opt in Opt::ALL {
            let button = gtk::ToggleButton::new();
            for c in ["w11-button", "w11-subtle", "alles-toggle"] {
                button.add_css_class(c);
            }
            button.set_valign(gtk::Align::Center);
            button.set_widget_name(&format!("alles-toggle-{}", opt.key()));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let glyph = match opt {
                Opt::MatchCase => art::text_glyph("Aa", ""),
                Opt::WholeWord => art::text_glyph("ab", "word"),
                Opt::MatchPath => kit::icons::glyph("folder-open", 16).upcast(),
                Opt::Regex => art::text_glyph(".*", "mono"),
            };
            glyph.set_valign(gtk::Align::Center);
            row.append(&glyph);
            let label = gtk::Label::new(None);
            label.add_css_class("alles-toggle-label");
            row.append(&label);
            button.set_child(Some(&row));
            group.append(&button);
            toggles.push(Toggle { opt, button, label });
        }
        cmd.append(&group);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        spacer.set_size_request(8, -1);
        cmd.append(&spacer);
        let sort_btn = cmd_button("sort", true);
        sort_btn.button.set_widget_name("alles-sort");
        cmd.append(&sort_btn.button);
        cmd.append(&sep());
        let (tips_btn, tips_label) = pane_button("help");
        tips_btn.set_widget_name("alles-tips");
        let (preview_btn, preview_label) = pane_button("view");
        preview_btn.set_widget_name("alles-preview");
        let options_btn = cmd_button("settings", false);
        options_btn.button.set_widget_name("alles-options");
        cmd.append(&tips_btn);
        cmd.append(&preview_btn);
        cmd.append(&options_btn.button);

        // ---- results, overlays, panes, status bar
        let model = Results::default();
        let table = ResultsTable::new(&model, &widths);
        let results = gtk::Overlay::new();
        results.add_css_class("alles-results");
        results.set_hexpand(true);
        results.set_vexpand(true);
        results.set_child(Some(table.widget()));
        let overlay_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        overlay_box.add_css_class("alles-overlay-box");
        overlay_box.set_halign(gtk::Align::Center);
        overlay_box.set_valign(gtk::Align::Start);
        let overlay_host = gtk::ScrolledWindow::new();
        overlay_host.add_css_class("alles-overlay");
        overlay_host.set_hscrollbar_policy(gtk::PolicyType::Never);
        overlay_host.set_child(Some(&overlay_box));
        overlay_host.set_visible(false);
        results.add_overlay(&overlay_host);
        let pane_host = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pane_host.add_css_class("alles-panehost");
        pane_host.set_visible(false);
        // the pane's width is set (size_panes); its children must not make it take the spare width
        pane_host.set_hexpand(false);
        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main.add_css_class("alles-main");
        main.set_vexpand(true);
        main.append(&results);
        main.append(&pane_host);

        let status = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        status.add_css_class("alles-status");
        status.set_widget_name("alles-status");
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left.add_css_class("alles-status-left");
        left.set_hexpand(true);
        let count = gtk::Label::new(None);
        count.add_css_class("alles-status-count");
        count.add_css_class("w11-tabular");
        let status_sep = || {
            let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            s.add_css_class("alles-status-sep");
            s.set_valign(gtk::Align::Center);
            s.set_visible(false);
            s.upcast::<gtk::Widget>()
        };
        let sel_sep = status_sep();
        let sel = gtk::Label::new(None);
        sel.add_css_class("w11-tabular");
        sel.set_visible(false);
        let ms_sep = status_sep();
        let ms = gtk::Label::new(None);
        ms.add_css_class("alles-status-ms");
        ms.add_css_class("w11-tabular");
        ms.set_visible(false);
        left.append(&count);
        left.append(&sel_sep);
        left.append(&sel);
        left.append(&ms_sep);
        left.append(&ms);
        let idx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        idx.add_css_class("alles-status-index");
        status.append(&left);
        status.append(&idx);

        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("alles-layer");
        layer.set_vexpand(true);
        layer.append(&main);
        layer.append(&status);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("alles");
        content.append(&crate::clamp::Clamp::shrink(&cmd));
        content.append(&layer);
        win.set_content(&content);

        // ---- the state
        let s = ctx.setting("alles");
        let sort = view["sort"].as_str().filter(|s| SORTS.contains(s)).unwrap_or("name").to_owned();
        let saved_pane = match view["pane"].as_str() {
            Some("preview") => Some("preview"),
            Some("tips") => Some("tips"),
            _ => None,
        };
        let query = match (&params.query, &params.path) {
            (Some(q), _) => q.clone(),
            (None, Some(p)) => format!("in:\"{p}\" "),
            _ => String::new(),
        };
        let filter_v = params.filter.clone().filter(|f| FILTERS.contains(&f.as_str())).unwrap_or_else(|| "all".into());
        filter.set_selected(Some(&filter_v));
        let state = State {
            query: query.clone(),
            filter: filter_v,
            opts: [s["matchCase"] == true, s["wholeWord"] == true, s["matchPath"] == true, s["regex"] == true],
            sort,
            desc: view["desc"] == true,
            pane: saved_pane,
            index_kind: "unset".into(),
            narrow: u8::MAX,
            ..Default::default()
        };
        let preview = PreviewPane::new(
            |e| {
                if let Some(app) = App::get() {
                    actions::open_item(app.parent(), e);
                }
            },
            |e| {
                if let Some(app) = App::get() {
                    actions::open_containing(app.parent(), e);
                }
            },
        );
        let app = Rc::new(App {
            ctx: ctx.clone(),
            win,
            search,
            clear,
            filter,
            toggles,
            sort_btn,
            tips_btn,
            tips_label,
            preview_btn,
            preview_label,
            options_btn,
            model,
            table,
            results,
            overlay_host,
            overlay_box,
            pane_host,
            preview,
            count,
            sel,
            sel_sep,
            ms,
            ms_sep,
            idx,
            st: RefCell::new(state),
            update_overlay: RefCell::new(None),
            update_index: RefCell::new(None),
            query_timer: RefCell::new(None),
            preview_timer: RefCell::new(None),
            changed_timer: RefCell::new(None),
            changed_last: Cell::new(None),
            width_timer: RefCell::new(None),
            silent: Cell::new(false),
            syncing: Cell::new(false),
        });
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
        app.silent.set(true);
        app.search.set_text(&query);
        app.silent.set(false);
        app.relabel_static();
        app.wire();
        app.sync_toggles();
        let (sort, desc) = {
            let s = app.st.borrow();
            (s.sort.clone(), s.desc)
        };
        app.table.set_sort(&sort, desc);
        let pane = match params.pane.as_deref() {
            Some("preview") => Some("preview"),
            Some("tips") => Some("tips"),
            Some("none") => None,
            _ => saved_pane,
        };
        app.set_pane(pane);
        app.render_counts();
        // No table until the index state and the first query decide between results and an overlay (show_overlay
        // shows it): the window opened with the bare column header, the welcome or first-run panel replaced it a
        // moment later, and the cairo renderer's damage missed the top pixel row of the header's text, which
        // stayed on screen as faint dots under the command bar.
        app.table.widget().set_visible(false);
        app.win.present();
        app.search.grab_focus();
        app.search.set_position(-1);
        let a = app.clone();
        app.refresh_status(move || {
            a.run_query(false, false);
            a.debug_open(&params);
        });
    }

    /// The running app (main thread).
    pub fn get() -> Option<Rc<App>> {
        APP.with(|a| a.borrow().clone())
    }

    /// A later launch handed to this process (a deep link): main.js frame.onAppNavigate.
    pub fn navigate(params: Params) {
        let Some(app) = App::get() else { return };
        app.win.present();
        if let Some(q) = &params.query {
            app.set_query(q, true);
        } else if let Some(p) = &params.path {
            app.set_query(&format!("in:\"{p}\" "), true);
        }
        if let Some(f) = params.filter.as_deref().filter(|f| FILTERS.contains(f)) {
            app.st.borrow_mut().filter = f.to_owned();
            app.filter.set_selected(Some(f));
            app.run_query(false, false);
        }
    }

    fn parent(&self) -> &gtk::Widget {
        self.win.content().upcast_ref()
    }

    // -------------------------------------------------------------------------------------------- wiring

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let me = move || weak.upgrade();

        // the search box
        let m = me.clone();
        self.search.connect_changed(move |s| {
            let Some(app) = m() else { return };
            app.clear.set_visible(s.has_focus() && !s.text().is_empty());
            if app.silent.get() {
                return;
            }
            {
                let mut st = app.st.borrow_mut();
                st.query = s.text().to_string();
                st.query_pending = true;
            }
            app.query_changed();
        });
        let m = me.clone();
        self.clear.connect_clicked(move |_| {
            if let Some(app) = m() {
                app.search.set_text("");
                app.search.grab_focus();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let m = me.clone();
        focus.connect_enter(move |_| {
            if let Some(app) = m() {
                app.clear.set_visible(!app.search.text().is_empty());
            }
        });
        let m = me.clone();
        focus.connect_leave(move |_| {
            if let Some(app) = m() {
                app.clear.set_visible(false);
            }
        });
        self.search.add_controller(focus);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let m = me.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(app) = m() else { return glib::Propagation::Proceed };
            if app.search_key(key, mods) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.search.add_controller(keys);

        // the command bar
        let m = me.clone();
        self.filter.connect_changed(move |v| {
            if let Some(app) = m() {
                app.st.borrow_mut().filter = v.to_owned();
                app.run_query(false, false);
            }
        });
        for tg in &self.toggles {
            let m = me.clone();
            let opt = tg.opt;
            // toggled, not clicked: the dev channel's "click" toggles a toggle button without a click
            tg.button.connect_toggled(move |b| {
                if let Some(app) = m()
                    && !app.syncing.get()
                    && b.is_active() != app.opt(opt)
                {
                    app.set_option(opt, b.is_active());
                }
            });
        }
        let m = me.clone();
        self.sort_btn.button.connect_clicked(move |b| {
            if let Some(app) = m() {
                app.open_sort_menu(b);
            }
        });
        let m = me.clone();
        self.tips_btn.connect_toggled(move |b| {
            if let Some(app) = m()
                && !app.syncing.get()
            {
                let p = app.st.borrow().pane;
                if b.is_active() {
                    app.set_pane(Some("tips"));
                } else if p == Some("tips") {
                    app.set_pane(None);
                }
            }
        });
        let m = me.clone();
        self.preview_btn.connect_toggled(move |b| {
            if let Some(app) = m()
                && !app.syncing.get()
            {
                let p = app.st.borrow().pane;
                if b.is_active() {
                    app.set_pane(Some("preview"));
                } else if p == Some("preview") {
                    app.set_pane(None);
                }
            }
        });
        let m = me.clone();
        self.options_btn.button.connect_clicked(move |_| {
            if let Some(app) = m() {
                app.open_options();
            }
        });

        // the table
        let m = me.clone();
        self.model.set_fetch(move |page, gen_id| {
            if let Some(app) = m() {
                app.fetch_page(page, gen_id);
            }
        });
        let m = me.clone();
        self.table.connect_sort(move |by| {
            if let Some(app) = m() {
                let (cur, desc) = {
                    let s = app.st.borrow();
                    (s.sort.clone(), s.desc)
                };
                if by == cur {
                    app.set_sort(by, !desc);
                } else {
                    app.set_sort(by, table::first_desc(by));
                }
            }
        });
        let m = me.clone();
        self.table.connect_widths(move |w| {
            if let Some(app) = m() {
                if let Some(t) = app.width_timer.borrow_mut().take() {
                    t.remove();
                }
                let a = Rc::downgrade(&app);
                let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
                    if let Some(app) = a.upgrade() {
                        app.width_timer.borrow_mut().take();
                    }
                    save_view(json!({ "widths": w }));
                });
                *app.width_timer.borrow_mut() = Some(id);
            }
        });
        let m = me.clone();
        self.table.view().connect_activate(move |_, pos| {
            if let Some(app) = m()
                && let Some(e) = app.model.entry(pos)
            {
                actions::open_item(app.parent(), &e);
            }
        });
        let m = me.clone();
        self.table.selection().connect_selection_changed(move |_, _, _| {
            if let Some(app) = m() {
                app.render_counts();
                app.update_preview_selection();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let m = me.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(app) = m() else { return glib::Propagation::Proceed };
            if app.table_key(key, mods) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.table.view().add_controller(keys);
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let m = me.clone();
        click.connect_pressed(move |g, _, x, y| {
            let Some(app) = m() else { return };
            let Some(i) = app.table.index_at(x, y) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            if !app.table.is_selected(i) {
                app.table.focus_row(i, true);
            }
            app.open_row_menu(i, x, y);
        });
        self.table.view().add_controller(click);
        let m = me.clone();
        self.win.window().connect_focus_widget_notify(move |w| {
            let Some(app) = m() else { return };
            let Some(f) = gtk::prelude::GtkWindowExt::focus(w) else { return };
            if let Some((i, _)) = app.table.row_of(&f) {
                let changed = app.st.borrow().focus != Some(i);
                if changed {
                    app.st.borrow_mut().focus = Some(i);
                    app.schedule_preview(false);
                }
            }
        });

        // window shortcuts (main.js document keydown; not while a dialog is open)
        for accel in ["<Control>f", "<Control>e", "<Control>l", "F3"] {
            let m = me.clone();
            self.win.add_shortcut(accel, move || {
                if let Some(app) = m() {
                    app.focus_search(true);
                }
                true
            });
        }
        let m = me.clone();
        self.win.add_shortcut("F5", move || {
            if let Some(app) = m() {
                app.run_query(true, false);
                app.refresh_status(|| {});
            }
            true
        });
        for (accel, opt) in [("<Alt>c", Opt::MatchCase), ("<Alt>w", Opt::WholeWord), ("<Alt>p", Opt::MatchPath), ("<Alt>r", Opt::Regex)] {
            let m = me.clone();
            self.win.add_shortcut(accel, move || {
                if let Some(app) = m() {
                    let on = app.opt(opt);
                    app.set_option(opt, !on);
                }
                true
            });
        }

        // live data
        let m = me.clone();
        self.ctx.on("index.status", move |_, data| {
            if let Some(app) = m() {
                app.apply_status(data.clone());
            }
        });
        let m = me.clone();
        self.ctx.on("index.changed", move |_, _| {
            if let Some(app) = m() {
                app.index_changed();
            }
        });
        let m = me.clone();
        self.ctx.on_settings(move |settings, path| {
            let Some(app) = m() else { return };
            let a = &settings["alles"];
            let mut changed = false;
            {
                let mut st = app.st.borrow_mut();
                for (i, opt) in Opt::ALL.iter().enumerate() {
                    let v = a[opt.key()] == true;
                    if st.opts[i] != v {
                        st.opts[i] = v;
                        changed = true;
                    }
                }
            }
            if changed {
                app.sync_toggles();
            }
            if path.is_none_or(|p| p.starts_with("alles.showHidden") || p == "alles") && app.active_query() {
                app.run_query(true, false);
            }
        });
        let m = me.clone();
        i18n::on_change(move || {
            if let Some(app) = m() {
                app.relabel();
            }
        });
        let m = me.clone();
        let win = self.win.window().clone();
        let connect = move |w: &gtk::ApplicationWindow| {
            let Some(surface) = w.surface() else { return };
            let m = m.clone();
            surface.connect_layout(move |_, width, _| {
                if let Some(app) = m() {
                    app.breakpoints(width);
                }
            });
        };
        if win.is_realized() {
            connect(&win);
        } else {
            win.connect_realize(move |w| connect(w));
        }
    }

    // ------------------------------------------------------------------------------------- texts, look

    fn relabel_static(&self) {
        for tg in &self.toggles {
            tg.label.set_label(tg.opt.label());
            tg.button.set_tooltip_text(Some(tg.opt.tip()));
        }
        self.sort_btn.label.set_label(t("Sorteren", "Sort"));
        self.tips_label.set_label(t("Zoektips", "Search tips"));
        self.preview_label.set_label(t("Voorbeeld", "Preview"));
        self.options_btn.label.set_label(t("Opties", "Options"));
        self.filter.widget().set_tooltip_text(Some(t("Soort", "Kind")));
    }

    /// A language switch: every text again (main.js reloaded the window and kept the search).
    fn relabel(self: &Rc<Self>) {
        self.relabel_static();
        self.filter.set_options(filter_options());
        self.table.relabel();
        self.preview.relabel();
        let pane = self.st.borrow().pane;
        if pane == Some("tips") {
            self.set_pane(Some("tips"));
        }
        let overlay = self.st.borrow_mut().overlay.take();
        *self.update_overlay.borrow_mut() = None;
        if let Some(o) = overlay {
            self.show_overlay(Some(o));
        }
        self.st.borrow_mut().index_kind = "unset".into();
        *self.update_index.borrow_mut() = None;
        self.render_index_state();
        self.render_counts();
        if self.active_query() {
            self.run_query(true, true);
        }
    }

    /// Narrow windows (alles.css @container): labels go, the panes narrow and then go.
    fn breakpoints(self: &Rc<Self>, width: i32) {
        let level = if width <= 620 {
            3
        } else if width <= 820 {
            2
        } else if width <= 1020 {
            1
        } else {
            0
        };
        if self.st.borrow().narrow == level {
            return;
        }
        self.st.borrow_mut().narrow = level;
        for tg in &self.toggles {
            tg.label.set_visible(level == 0);
            if level == 0 {
                tg.button.remove_css_class("alles-narrow");
            } else {
                tg.button.add_css_class("alles-narrow");
            }
        }
        for (b, l) in [
            (self.sort_btn.button.upcast_ref::<gtk::Widget>(), &self.sort_btn.label),
            (self.tips_btn.upcast_ref(), &self.tips_label),
            (self.preview_btn.upcast_ref(), &self.preview_label),
            (self.options_btn.button.upcast_ref(), &self.options_btn.label),
        ] {
            l.set_visible(level < 2);
            if level < 2 {
                b.remove_css_class("alles-narrow");
            } else {
                b.add_css_class("alles-narrow");
            }
        }
        self.size_panes();
        self.render_counts();
    }

    fn size_panes(&self) {
        let (pane, level) = {
            let s = self.st.borrow();
            (s.pane, s.narrow)
        };
        let narrow = level >= 2 && level != u8::MAX;
        let w = match pane {
            Some("tips") => if narrow { 280 } else { 340 },
            _ => if narrow { 260 } else { 300 },
        };
        self.pane_host.set_size_request(w, -1);
        self.pane_host.set_visible(pane.is_some() && !(level == 3));
    }

    fn opt(&self, o: Opt) -> bool {
        let i = Opt::ALL.iter().position(|x| *x == o).unwrap_or(0);
        self.st.borrow().opts[i]
    }

    fn sync_toggles(&self) {
        let opts = self.st.borrow().opts;
        self.syncing.set(true);
        for (i, tg) in self.toggles.iter().enumerate() {
            if tg.button.is_active() != opts[i] {
                tg.button.set_active(opts[i]);
            }
        }
        self.syncing.set(false);
    }

    fn active_query(&self) -> bool {
        let s = self.st.borrow();
        !s.query.trim().is_empty() || s.filter != "all"
    }

    fn focus_search(&self, select: bool) {
        self.search.grab_focus();
        if select {
            self.search.select_region(0, -1);
        } else {
            self.search.set_position(-1);
        }
    }

    // ---------------------------------------------------------------------------------------- panes

    fn set_pane(self: &Rc<Self>, pane: Option<&'static str>) {
        self.st.borrow_mut().pane = pane;
        controls::clear(&self.pane_host);
        match pane {
            Some("preview") => {
                let w = self.preview.widget();
                w.set_hexpand(true);
                self.pane_host.append(&w);
                self.schedule_preview(true);
            }
            Some("tips") => {
                let weak = Rc::downgrade(self);
                let weak2 = Rc::downgrade(self);
                let w = panes::tips_pane(
                    move |q| {
                        if let Some(app) = weak.upgrade() {
                            app.set_query(q, true);
                        }
                    },
                    move || {
                        if let Some(app) = weak2.upgrade() {
                            app.set_pane(None);
                        }
                    },
                );
                w.set_hexpand(true);
                self.pane_host.append(&w);
            }
            _ => {}
        }
        self.size_panes();
        self.syncing.set(true);
        self.tips_btn.set_active(pane == Some("tips"));
        self.preview_btn.set_active(pane == Some("preview"));
        self.syncing.set(false);
        let (sort, desc) = {
            let s = self.st.borrow();
            (s.sort.clone(), s.desc)
        };
        save_view(json!({ "sort": sort, "desc": desc, "pane": pane }));
    }

    fn focused_entry(&self) -> Option<Rc<Entry>> {
        let f = self.focus_index()?;
        self.model.entry(f)
    }

    /// The row with the keyboard focus (the last one, while the focus is elsewhere).
    fn focus_index(&self) -> Option<u32> {
        if let Some(f) = gtk::prelude::GtkWindowExt::focus(self.win.window())
            && let Some((i, _)) = self.table.row_of(&f)
        {
            self.st.borrow_mut().focus = Some(i);
            return Some(i);
        }
        self.st.borrow().focus.filter(|i| *i < self.model.total())
    }

    fn schedule_preview(self: &Rc<Self>, now: bool) {
        if self.st.borrow().pane != Some("preview") {
            return;
        }
        if let Some(t) = self.preview_timer.borrow_mut().take() {
            t.remove();
        }
        let run = |app: &Rc<App>| {
            let n = app.table.selected_count();
            if n > 1 {
                app.update_preview_selection();
            } else {
                app.preview.show(app.focused_entry().as_deref());
            }
        };
        if now {
            run(self);
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(70), move || {
            if let Some(app) = weak.upgrade() {
                app.preview_timer.borrow_mut().take();
                run(&app);
            }
        });
        *self.preview_timer.borrow_mut() = Some(id);
    }

    fn update_preview_selection(self: &Rc<Self>) {
        if self.st.borrow().pane != Some("preview") {
            return;
        }
        let n = self.table.selected_count();
        if n > 1 {
            let idx = self.table.selected_indices(5000);
            let mut size = 0.0;
            let mut known = true;
            for i in &idx {
                match self.model.entry(*i) {
                    Some(e) => size += e.size,
                    None => known = false,
                }
            }
            self.preview.show_many(n, (known && idx.len() as u64 == n).then_some(size));
        } else {
            self.schedule_preview(false);
        }
    }

    // ------------------------------------------------------------------------------- options, sorting

    fn set_option(self: &Rc<Self>, opt: Opt, on: bool) {
        let i = Opt::ALL.iter().position(|x| *x == opt).unwrap_or(0);
        self.st.borrow_mut().opts[i] = on;
        self.sync_toggles();
        self.ctx.set_setting(&format!("alles.{}", opt.key()), json!(on));
        if !self.st.borrow().query.trim().is_empty() {
            self.run_query(false, false);
        }
    }

    fn set_sort(self: &Rc<Self>, by: &str, desc: bool) {
        {
            let mut s = self.st.borrow_mut();
            s.sort = by.to_owned();
            s.desc = desc;
        }
        self.table.set_sort(by, desc);
        let pane = self.st.borrow().pane;
        save_view(json!({ "sort": by, "desc": desc, "pane": pane }));
        self.run_query(false, false);
    }

    /// A sort that is not saved as the view (the quick chips).
    fn quick_sort(&self, by: &str, desc: bool) {
        {
            let mut s = self.st.borrow_mut();
            s.sort = by.to_owned();
            s.desc = desc;
        }
        self.table.set_sort(by, desc);
    }

    fn open_sort_menu(self: &Rc<Self>, anchor: &gtk::Button) {
        let (sort, desc) = {
            let s = self.st.borrow();
            (s.sort.clone(), s.desc)
        };
        let mut items: Vec<MenuItem> = SORTS
            .iter()
            .map(|k| {
                let weak = Rc::downgrade(self);
                let key = (*k).to_owned();
                let (cur, d) = (sort.clone(), desc);
                MenuItem::new(text::column_label(k), move || {
                    if let Some(app) = weak.upgrade() {
                        let desc = if key == cur { d } else { key == "size" || key == "modified" };
                        app.set_sort(&key, desc);
                    }
                })
                .radio(sort == *k)
                .name(&format!("alles-sort-{k}"))
            })
            .collect();
        items.push(MenuItem::separator());
        for (label, d) in [(t("Oplopend", "Ascending"), false), (t("Aflopend", "Descending"), true)] {
            let weak = Rc::downgrade(self);
            let s = sort.clone();
            items.push(
                MenuItem::new(label, move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_sort(&s, d);
                    }
                })
                .radio(desc == d),
            );
        }
        controls::show_menu_for(anchor, items);
    }

    fn open_options(self: &Rc<Self>) {
        let status = self.st.borrow().status.clone().unwrap_or(Value::Null);
        let weak = Rc::downgrade(self);
        options::open_options(self.parent(), status, move || {
            if let Some(app) = weak.upgrade() {
                app.refresh_status(|| {});
            }
        });
    }

    // ------------------------------------------------------------------------------------- searching

    fn base_args(&self) -> Value {
        let s = self.st.borrow();
        json!({
            "query": s.query, "filter": s.filter, "sort": s.sort, "desc": s.desc,
            "matchCase": s.opts[0], "wholeWord": s.opts[1], "matchPath": s.opts[2], "regex": s.opts[3],
            "channel": "alles",
        })
    }

    fn query_changed(self: &Rc<Self>) {
        if let Some(t) = self.query_timer.borrow_mut().take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(30), move || {
            if let Some(app) = weak.upgrade() {
                app.query_timer.borrow_mut().take();
                app.run_query(false, false);
            }
        });
        *self.query_timer.borrow_mut() = Some(id);
    }

    fn is_first_run(&self) -> bool {
        let s = self.st.borrow();
        s.status.as_ref().is_some_and(|st| {
            st["state"] == "building" && st["files"].as_f64().unwrap_or(0.0) == 0.0 && st["folders"].as_f64().unwrap_or(0.0) == 0.0
        })
    }

    fn run_query(self: &Rc<Self>, keep_scroll: bool, keep_selection: bool) {
        let (my, q, empty) = {
            let mut s = self.st.borrow_mut();
            s.seq += 1;
            (s.seq, s.query.trim().to_owned(), s.query.trim().is_empty() && s.filter == "all")
        };
        if empty {
            {
                let mut s = self.st.borrow_mut();
                s.query_pending = false;
                s.pending_enter = None;
                s.args = None;
                s.result = None;
                s.focus = None;
            }
            self.table.set_highlight(None);
            self.model.reset(0, Vec::new(), false);
            self.show_overlay(Some(if self.is_first_run() { Overlay::First } else { Overlay::Welcome }));
            self.render_counts();
            return;
        }
        if self.is_first_run() {
            self.show_overlay(Some(Overlay::First));
        }
        let args = self.base_args();
        let mut call = args.clone();
        call["offset"] = json!(0);
        call["limit"] = json!(PAGE);
        let weak = Rc::downgrade(self);
        self.ctx.call("index.search", call, move |r| {
            let Some(app) = weak.upgrade() else { return };
            if app.st.borrow().seq != my {
                return;
            }
            match r {
                Ok(r) => app.query_done(args, &r, &q, keep_scroll, keep_selection),
                Err(e) => app.query_failed(&e, &q),
            }
        });
    }

    fn query_done(self: &Rc<Self>, args: Value, r: &Value, q: &str, keep_scroll: bool, keep_selection: bool) {
        let total = r["total"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
        let took = r["tookMs"].as_f64();
        let match_path = self.opt(Opt::MatchPath);
        let pending_enter = {
            let mut s = self.st.borrow_mut();
            s.query_pending = false;
            s.args = Some(args);
            s.result = Some((total, took));
            if !(keep_scroll || keep_selection) {
                s.focus = None;
            }
            s.pending_enter.take()
        };
        self.table.set_highlight(Highlight::from_value(&r["highlight"], match_path || has_path_modifier(q)));
        let keep = keep_scroll || keep_selection;
        self.model.reset(total, model::entries_of(r), keep);
        if keep && !keep_selection {
            self.table.selection().unselect_all();
        }
        if self.is_first_run() {
            self.show_overlay(Some(Overlay::First));
        } else if total == 0 {
            self.show_overlay(Some(Overlay::Empty(q.to_owned())));
        } else {
            self.show_overlay(None);
        }
        self.render_counts();
        self.schedule_preview(false);
        if let Some(ctrl) = pending_enter {
            self.enter_in_search(ctrl);
        }
    }

    fn query_failed(self: &Rc<Self>, e: &kit::CallError, q: &str) {
        let had_result = {
            let mut s = self.st.borrow_mut();
            s.query_pending = false;
            s.pending_enter = None;
            s.result.is_some()
        };
        if e.code == "cancelled" {
            return;
        }
        let bad = e.code == "bad_query" || e.code == "bad_regex";
        if bad && still_typing(q) && had_result {
            return;
        }
        {
            let mut s = self.st.borrow_mut();
            s.args = None;
            s.result = None;
            s.focus = None;
        }
        self.model.reset(0, Vec::new(), false);
        if bad {
            self.show_overlay(Some(Overlay::QueryError(e.message.clone())));
        } else {
            let msg = if e.code == "index_unavailable" || e.code == "index_restarting" { String::new() } else { e.message.clone() };
            self.show_overlay(Some(Overlay::Unavailable(msg)));
        }
        self.render_counts();
    }

    fn fetch_page(self: &Rc<Self>, page: u32, gen_id: u64) {
        let Some(mut args) = self.st.borrow().args.clone() else {
            self.model.page_loaded(gen_id, page, None);
            return;
        };
        args["offset"] = json!(page * PAGE);
        args["limit"] = json!(PAGE);
        let weak = Rc::downgrade(self);
        self.ctx.call("index.search", args, move |r| {
            let Some(app) = weak.upgrade() else { return };
            match r {
                Ok(v) => {
                    app.model.page_loaded(gen_id, page, Some(model::entries_of(&v)));
                    let focus = app.st.borrow().focus;
                    if focus.is_some_and(|f| f / PAGE == page) {
                        app.schedule_preview(false);
                    }
                    if app.table.selected_count() > 1 {
                        app.update_preview_selection();
                    }
                }
                Err(e) => {
                    if e.code != "cancelled" {
                        eprintln!("win11-gtk-alles: page {page}: {e}");
                    }
                    app.model.page_loaded(gen_id, page, None);
                }
            }
        });
    }

    /// The results at these indices (fetching pages not there yet), in order; `done` gets what it could.
    fn get_items(self: &Rc<Self>, indices: Vec<u32>, done: impl FnOnce(Vec<Rc<Entry>>) + 'static) {
        let gen_id = self.model.generation();
        let args = self.st.borrow().args.clone();
        let mut need: Vec<u32> = indices.iter().map(|i| i / PAGE).filter(|p| !self.model.has_page(*p)).collect();
        need.dedup();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut fetched: HashMap<u32, Vec<Rc<Entry>>> = HashMap::new();
            if let Some(args) = args {
                let ctx = kit::ctx();
                for p in need {
                    let mut a = args.clone();
                    a["offset"] = json!(p * PAGE);
                    a["limit"] = json!(PAGE);
                    if let Ok(v) = ctx.call_async("index.search", a).await {
                        fetched.insert(p, model::entries_of(&v));
                    }
                }
            }
            let Some(app) = weak.upgrade() else { return };
            if app.model.generation() != gen_id {
                done(Vec::new());
                return;
            }
            let mut out = Vec::with_capacity(indices.len());
            for i in &indices {
                let e = app.model.entry(*i).or_else(|| fetched.get(&(i / PAGE)).and_then(|p| p.get((i % PAGE) as usize).cloned()));
                if let Some(e) = e.filter(|e| !e.path.is_empty()) {
                    out.push(e);
                }
            }
            for (p, entries) in fetched {
                app.model.store_page(gen_id, p, entries);
            }
            done(out);
        });
    }

    fn set_query(self: &Rc<Self>, q: &str, focus: bool) {
        self.st.borrow_mut().query = q.to_owned();
        self.silent.set(true);
        self.search.set_text(q);
        self.silent.set(false);
        self.run_query(false, false);
        if focus {
            self.focus_search(false);
        }
    }

    /// Enter in the search box: to the first result (Ctrl+Enter opens its folder).
    fn enter_in_search(self: &Rc<Self>, ctrl: bool) -> bool {
        if self.model.total() == 0 {
            return false;
        }
        if ctrl && let Some(e) = self.focused_entry().or_else(|| self.model.entry(0)) {
            actions::open_containing(self.parent(), &e);
            return true;
        }
        self.focus_table();
        true
    }

    fn focus_table(&self) {
        match self.st.borrow().focus.filter(|f| *f < self.model.total()) {
            Some(f) => self.table.focus_row(f, false),
            None => self.table.focus_row(0, true),
        }
    }

    fn search_key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        match key {
            gdk::Key::Down | gdk::Key::Page_Down | gdk::Key::KP_Down => {
                if self.model.total() > 0 {
                    self.focus_table();
                    return true;
                }
                false
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                let pending = {
                    let s = self.st.borrow();
                    s.query_pending && !self.search.text().trim().is_empty()
                };
                if pending {
                    self.st.borrow_mut().pending_enter = Some(ctrl);
                    return true;
                }
                self.enter_in_search(ctrl)
            }
            gdk::Key::Escape => {
                if !self.search.text().is_empty() {
                    self.set_query("", false);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn table_key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter);
        if enter && ctrl {
            if let Some(e) = self.focused_entry() {
                actions::open_containing(self.parent(), &e);
            }
            return true;
        }
        if enter && alt {
            if let Some(e) = self.focused_entry() {
                actions::show_properties(self.parent(), &e);
            }
            return true;
        }
        if enter {
            if let Some(e) = self.focused_entry() {
                actions::open_item(self.parent(), &e);
            }
            return true;
        }
        if ctrl && matches!(key, gdk::Key::c | gdk::Key::C) {
            self.copy_selection(shift);
            return true;
        }
        if key == gdk::Key::Delete || key == gdk::Key::KP_Delete {
            self.trash_selection();
            return true;
        }
        if key == gdk::Key::Escape {
            self.focus_search(true);
            return true;
        }
        if matches!(key, gdk::Key::Up | gdk::Key::KP_Up) && !shift && self.focus_index().is_none_or(|i| i == 0) {
            self.focus_search(false);
            return true;
        }
        if key == gdk::Key::Menu || (key == gdk::Key::F10 && shift) {
            if let Some(f) = gtk::prelude::GtkWindowExt::focus(self.win.window())
                && let Some((i, rect)) = self.table.row_of(&f)
            {
                self.open_row_menu(i, rect.x() as f64 + 48.0, rect.y() as f64 + rect.height() as f64 / 2.0);
                return true;
            }
            return false;
        }
        if !ctrl && !alt
            && let Some(ch) = key.to_unicode()
            && !ch.is_control()
            && ch != ' '
        {
            self.focus_search(false);
            let pos = self.search.text().chars().count() as i32;
            let mut p = pos;
            self.search.insert_text(&ch.to_string(), &mut p);
            self.search.set_position(-1);
            return true;
        }
        false
    }

    // ------------------------------------------------------------------------------- menu, actions

    fn selected_items(self: &Rc<Self>, limit: usize, done: impl FnOnce(Vec<Rc<Entry>>) + 'static) {
        let mut idx = self.table.selected_indices(limit);
        if idx.is_empty()
            && let Some(f) = self.focus_index()
        {
            idx.push(f);
        }
        self.get_items(idx, done);
    }

    fn copy_selection(self: &Rc<Self>, names: bool) {
        let n = self.table.selected_count() as usize;
        let weak = Rc::downgrade(self);
        self.selected_items(COPY_MAX, move |items| {
            if let Some(app) = weak.upgrade() {
                actions::copy_paths(app.parent(), &items, names, n > COPY_MAX);
            }
        });
    }

    fn trash_selection(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.selected_items(actions::TRASH_MAX + 1, move |items| {
            let Some(app) = weak.upgrade() else { return };
            let w = Rc::downgrade(&app);
            actions::trash_items(app.parent(), items, move |moved| {
                if moved && let Some(app) = w.upgrade() {
                    app.run_query(true, false);
                }
            });
        });
    }

    fn open_row_menu(self: &Rc<Self>, index: u32, x: f64, y: f64) {
        let Some(item) = self.model.entry(index) else { return };
        let multi = self.table.selected_count().max(1) > 1;
        let weak = Rc::downgrade(self);
        let act = |f: Rc<dyn Fn(&Rc<App>, &Entry)>| {
            let w = weak.clone();
            let e = item.clone();
            move || {
                if let Some(app) = w.upgrade() {
                    f(&app, &e);
                }
            }
        };
        let copy: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, _| a.copy_selection(false));
        let names: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, _| a.copy_selection(true));
        let props: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, e| actions::show_properties(a.parent(), e));
        let trash: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, _| a.trash_selection());
        let open: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, e| actions::open_item(a.parent(), e));
        let folder: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, e| actions::open_containing(a.parent(), e));
        let explorer: Rc<dyn Fn(&Rc<App>, &Entry)> = Rc::new(|a, e| actions::open_in_explorer(a.parent(), e));
        let top = vec![
            TopAction::new("copy", t("Pad kopiëren", "Copy path"), act(copy.clone())),
            TopAction::new("properties", t("Eigenschappen", "Properties"), act(props.clone())).enabled(!multi),
            TopAction::new("delete", t("Naar Prullenbak", "Move to Recycle Bin"), act(trash.clone())).danger(),
        ];
        let mut items = vec![
            MenuItem::new(t("Openen", "Open"), act(open)).glyph("open").accel("Enter").name("alles-menu-open"),
            MenuItem::new(t("Map met bestand openen", "Open containing folder"), act(folder)).glyph("folder-open").accel("Ctrl+Enter").name("alles-menu-folder"),
        ];
        if item.is_dir {
            items.push(MenuItem::new(t("Openen in Verkenner", "Open in File Explorer"), act(explorer)).glyph("folder").name("alles-menu-explorer"));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new(t("Pad kopiëren", "Copy path"), act(copy)).glyph("copy").accel("Ctrl+C").name("alles-menu-copy"));
        items.push(MenuItem::new(t("Naam kopiëren", "Copy name"), act(names)).glyph("rename").accel("Ctrl+Shift+C").name("alles-menu-copyname"));
        items.push(MenuItem::separator());
        items.push(MenuItem::new(t("Eigenschappen", "Properties"), act(props)).glyph("properties").accel("Alt+Enter").enabled(!multi).name("alles-menu-properties"));
        items.push(MenuItem::separator());
        items.push(MenuItem::new(t("Naar Prullenbak", "Move to Recycle Bin"), act(trash)).glyph("delete").accel("Delete").danger().name("alles-menu-trash"));
        controls::show_menu_with_top(self.table.view(), x, y, top, items);
    }

    // ---------------------------------------------------------------------------------------- overlays

    fn show_overlay(self: &Rc<Self>, kind: Option<Overlay>) {
        // Status events update the panel in place (replacing it would reset the progress bar).
        {
            let cur = self.st.borrow().overlay.clone();
            if let (Some(c), Some(k)) = (&cur, &kind)
                && c.same_kind(k)
                && matches!(k, Overlay::Welcome | Overlay::First)
                && let Some(u) = self.update_overlay.borrow().clone()
            {
                u(self);
                return;
            }
        }
        self.st.borrow_mut().overlay = kind.clone();
        *self.update_overlay.borrow_mut() = None;
        let blocking = kind.as_ref().is_some_and(|k| !matches!(k, Overlay::Empty(_)));
        self.table.widget().set_visible(!blocking);
        controls::clear(&self.overlay_box);
        let Some(kind) = kind else {
            self.overlay_host.set_visible(false);
            self.results.remove_css_class("alles-has-overlay");
            self.results.remove_css_class("alles-has-empty");
            return;
        };
        self.overlay_host.set_visible(true);
        if matches!(kind, Overlay::Empty(_)) {
            self.results.add_css_class("alles-has-empty");
            self.results.remove_css_class("alles-has-overlay");
            self.overlay_host.set_margin_top(32);
        } else {
            self.results.add_css_class("alles-has-overlay");
            self.results.remove_css_class("alles-has-empty");
            self.overlay_host.set_margin_top(0);
        }
        let state = gtk::Box::new(gtk::Orientation::Vertical, 6);
        state.add_css_class("alles-state");
        state.set_halign(gtk::Align::Center);
        let wide = matches!(kind, Overlay::QueryError(_) | Overlay::Unavailable(_));
        self.overlay_box.set_halign(if wide { gtk::Align::Fill } else { gtk::Align::Center });
        let title = |text: &str, small: bool| {
            let l = controls::label(text, if small { "alles-state-title alles-small" } else { "alles-state-title" }, true);
            l.set_justify(gtk::Justification::Center);
            l.set_xalign(0.5);
            l.set_max_width_chars(60);
            l
        };
        let body_label = |text: &str| {
            let l = controls::label(text, "alles-state-body", true);
            l.set_justify(gtk::Justification::Center);
            l.set_xalign(0.5);
            l.set_max_width_chars(70);
            l
        };
        match &kind {
            Overlay::Welcome => {
                state.add_css_class("alles-welcome");
                state.append(&art::welcome());
                state.append(&title(t("Vind alles, meteen", "Find anything, instantly"), false));
                let body = body_label("");
                state.append(&body);
                let b = body.clone();
                let update: Update = Rc::new(move |app: &Rc<App>| {
                    let st = app.st.borrow().status.clone().unwrap_or(Value::Null);
                    let n = (st["files"].as_f64().unwrap_or(0.0) + st["folders"].as_f64().unwrap_or(0.0)) as u64;
                    b.set_label(&if n > 0 {
                        text::welcome_body(n)
                    } else {
                        t("Typ een deel van een naam. Alles zoekt terwijl je typt.", "Type part of a name. Alles searches as you type.").to_owned()
                    });
                });
                *self.update_overlay.borrow_mut() = Some(update);
                let l = controls::label(t("Snel zoeken", "Quick searches"), "alles-state-label", false);
                l.set_halign(gtk::Align::Center);
                state.append(&l);
                let quick = crate::wrap::WrapBox::new(8, 8);
                quick.add_css_class("alles-quick");
                let chips: [(&str, &str, &str, u8); 7] = [
                    ("image", t("Afbeeldingen", "Pictures"), "pictures", 0),
                    ("music", t("Muziek", "Music"), "music", 1),
                    ("video", t("Video's", "Videos"), "videos", 2),
                    ("document", t("Documenten", "Documents"), "documents", 3),
                    ("disk", t("Grote bestanden", "Large files"), "big", 4),
                    ("clock", t("Vandaag gewijzigd", "Changed today"), "today", 5),
                    ("download", t("Downloads", "Downloads"), "downloads", 6),
                ];
                for (glyph, label, name, which) in chips {
                    let b = gtk::Button::new();
                    b.add_css_class("alles-chip");
                    b.set_widget_name(&format!("alles-quick-{name}"));
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.append(&kit::icons::glyph(glyph, 16));
                    row.append(&gtk::Label::new(Some(label)));
                    b.set_child(Some(&row));
                    let weak = Rc::downgrade(self);
                    b.connect_clicked(move |_| {
                        if let Some(app) = weak.upgrade() {
                            app.quick(which);
                        }
                    });
                    quick.append(&b);
                }
                state.append(&quick);
                let link = controls::hyperlink(t("Zoektips bekijken", "View search tips"), None);
                if let Some(row) = link.child().and_downcast::<gtk::Box>() {
                    row.prepend(&kit::icons::glyph("help", 16));
                }
                link.set_halign(gtk::Align::Center);
                link.set_widget_name("alles-show-tips");
                let weak = Rc::downgrade(self);
                link.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.set_pane(Some("tips"));
                    }
                });
                state.append(&link);
            }
            Overlay::First => {
                state.add_css_class("alles-first");
                state.append(&art::welcome());
                state.append(&title(t("Alles maakt een overzicht van je bestanden", "Alles is building an overview of your files"), false));
                state.append(&body_label(&format!(
                    "{} {}",
                    t("Dit gebeurt één keer en duurt een paar minuten.", "This happens once and takes a few minutes."),
                    t(
                        "Daarna vind je elk bestand en elke map in een oogwenk, terwijl je typt.",
                        "After that, you’ll find every file and folder instantly as you type."
                    )
                )));
                let bar = ProgressBar::new();
                bar.set_indeterminate(true);
                bar.widget().add_css_class("alles-first-bar");
                bar.widget().set_size_request(380, -1);
                bar.widget().set_halign(gtk::Align::Center);
                state.append(bar.widget());
                let count = gtk::Box::new(gtk::Orientation::Horizontal, 16);
                count.add_css_class("alles-first-count");
                count.set_size_request(380, -1);
                count.set_halign(gtk::Align::Center);
                let found = controls::label("", "alles-first-found w11-tabular", false);
                found.set_hexpand(true);
                let pct = controls::label("", "alles-first-pct w11-tabular", false);
                count.append(&found);
                count.append(&pct);
                state.append(&count);
                let path = gtk::Label::new(Some("\u{a0}"));
                path.add_css_class("alles-first-path");
                path.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                path.set_single_line_mode(true);
                path.set_xalign(0.0);
                path.set_size_request(380, -1);
                path.set_max_width_chars(1);
                path.set_halign(gtk::Align::Center);
                state.append(&path);
                let note = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                note.add_css_class("alles-first-note");
                note.set_halign(gtk::Align::Center);
                let ni = kit::icons::glyph("info", 14);
                ni.set_valign(gtk::Align::Start);
                note.append(&ni);
                let nl = controls::label(
                    t(
                        "Indexeren stopt als je dit venster sluit en gaat verder wanneer je Alles weer opent.",
                        "Indexing stops when you close this window and continues when you open Alles again.",
                    ),
                    "",
                    true,
                );
                nl.set_max_width_chars(52);
                note.append(&nl);
                state.append(&note);
                let update: Update = Rc::new(move |app: &Rc<App>| {
                    let st = app.st.borrow().status.clone().unwrap_or(Value::Null);
                    let pr = &st["progress"];
                    match pr["percent"].as_f64() {
                        Some(p) => bar.set_fraction(p / 100.0),
                        None => bar.set_indeterminate(true),
                    }
                    found.set_label(&text::first_found(pr["done"].as_f64().unwrap_or(0.0) as u64));
                    match pr["percent"].as_f64() {
                        Some(p) => {
                            pct.set_label(&format!("{}%", text::num(p)));
                            pct.set_visible(true);
                        }
                        None => pct.set_visible(false),
                    }
                    // one line kept while the worker moves between folders
                    match pr["current"].as_str().filter(|s| !s.is_empty()) {
                        Some(c) => {
                            path.set_label(c);
                            path.set_opacity(1.0);
                            path.set_tooltip_text(Some(c));
                        }
                        None => {
                            path.set_label("\u{a0}");
                            path.set_opacity(0.0);
                            path.set_tooltip_text(None);
                        }
                    }
                });
                *self.update_overlay.borrow_mut() = Some(update);
            }
            Overlay::Empty(q) => {
                state.add_css_class("alles-empty");
                let icon = gtk::Box::new(gtk::Orientation::Vertical, 0);
                icon.add_css_class("alles-empty-icon");
                icon.set_halign(gtk::Align::Center);
                icon.set_size_request(72, 72);
                let g = kit::icons::glyph_stroke("search", 40, 1.2);
                g.set_vexpand(true);
                g.set_valign(gtk::Align::Center);
                icon.append(&g);
                state.append(&icon);
                state.append(&title(t("Niets gevonden", "Nothing found"), true));
                let (filter, match_path, regex) = {
                    let s = self.st.borrow();
                    (s.filter.clone(), s.opts[2], s.opts[3])
                };
                let body = if filter != "all" {
                    let f = text::filter_label(&filter);
                    if i18n::en() { format!("No items of kind “{f}” match “{q}”.") } else { format!("Er is niets van de soort “{f}” dat past bij “{q}”.") }
                } else if i18n::en() {
                    format!("No file or folder matches “{q}”.")
                } else {
                    format!("Er is geen bestand of map die past bij “{q}”.")
                };
                state.append(&body_label(&body));
                let hints = gtk::Box::new(gtk::Orientation::Vertical, 2);
                hints.add_css_class("alles-empty-hints");
                let hint = |parts: Vec<gtk::Widget>| {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                    row.set_halign(gtk::Align::Center);
                    row.add_css_class("alles-empty-hint");
                    for p in parts {
                        row.append(&p);
                    }
                    row
                };
                let plain = |s: &str| controls::label(s, "", false).upcast::<gtk::Widget>();
                hints.append(&hint(vec![plain(t(
                    "Controleer de spelling, of gebruik minder of kortere woorden.",
                    "Check the spelling, or use fewer or shorter words.",
                ))]));
                if !match_path && !regex {
                    let link = controls::hyperlink(t("Pad aanzetten", "Turn on Path"), None);
                    link.set_widget_name("alles-hint-path");
                    let weak = Rc::downgrade(self);
                    link.connect_clicked(move |_| {
                        if let Some(app) = weak.upgrade() {
                            app.set_option(Opt::MatchPath, true);
                        }
                    });
                    hints.append(&hint(vec![
                        plain(t("Zet ‘Pad’ aan om ook in mapnamen te zoeken.", "Turn on ‘Path’ to search folder names too.")),
                        link.upcast(),
                    ]));
                }
                if filter != "all" {
                    let link = controls::hyperlink(t("Alles", "Everything"), None);
                    link.set_widget_name("alles-hint-all");
                    let weak = Rc::downgrade(self);
                    link.connect_clicked(move |_| {
                        if let Some(app) = weak.upgrade() {
                            app.st.borrow_mut().filter = "all".into();
                            app.filter.set_selected(Some("all"));
                            app.run_query(false, false);
                        }
                    });
                    hints.append(&hint(vec![plain(t("Kies ‘Alles’ als soort.", "Choose ‘Everything’ as the kind.")), link.upcast()]));
                }
                if self.ctx.setting("alles.showHidden") != true {
                    let link = controls::hyperlink(t("Opties", "Options"), None);
                    link.set_widget_name("alles-hint-options");
                    let weak = Rc::downgrade(self);
                    link.connect_clicked(move |_| {
                        if let Some(app) = weak.upgrade() {
                            app.open_options();
                        }
                    });
                    let parts = if i18n::en() {
                        vec![plain("Hidden files aren’t shown. You can turn this on in"), link.upcast(), plain(".")]
                    } else {
                        vec![plain("Verborgen bestanden worden niet getoond. Je kunt dit aanzetten bij"), link.upcast()]
                    };
                    hints.append(&hint(parts));
                }
                state.append(&hints);
            }
            Overlay::QueryError(msg) => {
                state.add_css_class("alles-qerror");
                let bar = InfoBar::new(Severity::Warning, t("Deze zoekopdracht klopt niet helemaal", "There’s a problem with this search"), msg);
                bar.set_closable(false);
                let weak = Rc::downgrade(self);
                bar.set_action(Some((
                    t("Zoektips", "Search tips"),
                    Rc::new(move || {
                        if let Some(app) = weak.upgrade() {
                            app.set_pane(Some("tips"));
                        }
                    }),
                )));
                state.set_halign(gtk::Align::Fill);
                state.append(&crate::clamp::Clamp::new(bar.widget(), 620));
            }
            Overlay::Unavailable(msg) => {
                state.add_css_class("alles-qerror");
                let message = if msg.is_empty() {
                    t(
                        "Het overzicht van je bestanden kon niet worden bereikt. Probeer het zo opnieuw.",
                        "The overview of your files couldn’t be reached. Try again in a moment.",
                    )
                    .to_owned()
                } else {
                    msg.clone()
                };
                let bar = InfoBar::new(Severity::Error, t("Alles is even niet beschikbaar", "Alles is unavailable right now"), &message);
                bar.set_closable(false);
                let weak = Rc::downgrade(self);
                bar.set_action(Some((
                    t("Opnieuw proberen", "Try again"),
                    Rc::new(move || {
                        if let Some(app) = weak.upgrade() {
                            let a = app.clone();
                            app.refresh_status(move || a.run_query(false, false));
                        }
                    }),
                )));
                state.set_halign(gtk::Align::Fill);
                state.append(&crate::clamp::Clamp::new(bar.widget(), 620));
            }
        }
        self.overlay_box.append(&state);
        let u = self.update_overlay.borrow().clone();
        if let Some(u) = u {
            u(self);
        }
    }

    /// The quick searches of the welcome panel.
    fn quick(self: &Rc<Self>, which: u8) {
        let set_filter = |app: &Rc<App>, f: &str| {
            app.st.borrow_mut().filter = f.to_owned();
            app.filter.set_selected(Some(f));
        };
        match which {
            0 => {
                set_filter(self, "picture");
                self.set_sort("modified", true);
            }
            1 => {
                set_filter(self, "audio");
                self.run_query(false, false);
            }
            2 => {
                set_filter(self, "video");
                self.run_query(false, false);
            }
            3 => {
                set_filter(self, "document");
                self.set_sort("modified", true);
            }
            4 => {
                self.quick_sort("size", true);
                self.set_query("file: size:>1gb", false);
            }
            5 => {
                self.quick_sort("modified", true);
                self.set_query(t("dm:vandaag", "dm:today"), false);
            }
            _ => {
                self.quick_sort("modified", true);
                self.set_query("parent:~/Downloads", false);
            }
        }
    }

    // -------------------------------------------------------------------------------------- status bar

    fn render_counts(&self) {
        let (n, took) = {
            let s = self.st.borrow();
            match s.result {
                Some((total, took)) => (total as u64, took),
                None if s.query.trim().is_empty() && s.filter == "all" => {
                    let st = s.status.clone().unwrap_or(Value::Null);
                    ((st["files"].as_f64().unwrap_or(0.0) + st["folders"].as_f64().unwrap_or(0.0)) as u64, None)
                }
                None => (0, None),
            }
        };
        self.count.set_label(&text::objects(n));
        let sel = self.table.selected_count();
        let sel_text = if sel > 0 {
            if i18n::en() { format!("{} selected", text::num(sel as f64)) } else { format!("{} geselecteerd", text::num(sel as f64)) }
        } else {
            String::new()
        };
        self.sel.set_label(&sel_text);
        self.sel.set_visible(sel > 0);
        self.sel_sep.set_visible(sel > 0);
        let narrow = self.st.borrow().narrow == 3;
        let ms = took.map(text::took_ms).unwrap_or_default();
        self.ms.set_label(&ms);
        self.ms.set_visible(!ms.is_empty() && !narrow);
        self.ms_sep.set_visible(!ms.is_empty() && !narrow);
    }

    fn render_index_state(self: &Rc<Self>) {
        let st = self.st.borrow().status.clone();
        let kind = match &st {
            None => "none".to_owned(),
            Some(s) if s["state"] == "building" => {
                format!("building:{}", if s["progress"]["percent"].is_number() { "determinate" } else { "indeterminate" })
            }
            Some(s) if s["refreshing"] == true => "refreshing".to_owned(),
            Some(s) if s["state"] == "ready" => format!("ready:{}", live_on(s)),
            Some(s) if s["state"] == "error" => "error".to_owned(),
            Some(_) => "idle".to_owned(),
        };
        let same = self.st.borrow().index_kind == kind;
        if same {
            let u = self.update_index.borrow().clone();
            if let Some(u) = u {
                u(self);
            }
            return;
        }
        self.st.borrow_mut().index_kind = kind.clone();
        *self.update_index.borrow_mut() = None;
        controls::clear(&self.idx);
        let Some(st) = st else { return };
        let node = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        node.add_css_class("alles-idx");
        let label = |s: &str| {
            let l = gtk::Label::new(Some(s));
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l
        };
        if st["state"] == "building" {
            node.add_css_class("alles-building");
            let ring = ProgressRing::new(16, st["progress"]["percent"].as_f64().map(|p| p / 100.0));
            node.append(ring.widget());
            let l = label("");
            node.append(&l);
            let n = node.clone();
            let update: Update = Rc::new(move |app: &Rc<App>| {
                let st = app.st.borrow().status.clone().unwrap_or(Value::Null);
                let p = st["progress"]["percent"].as_f64();
                if let Some(p) = p {
                    ring.set_fraction(Some(p / 100.0));
                }
                l.set_label(&match p {
                    Some(p) => format!("{} {}%", t("Indexeren…", "Indexing…"), text::num(p)),
                    None => t("Indexeren…", "Indexing…").to_owned(),
                });
                match st["progress"]["current"].as_str().filter(|s| !s.is_empty()) {
                    Some(c) => n.set_tooltip_text(Some(&if i18n::en() { format!("Reading {c}") } else { format!("Bezig met {c}") })),
                    None => n.set_tooltip_text(None),
                }
            });
            *self.update_index.borrow_mut() = Some(update);
        } else if st["refreshing"] == true {
            node.add_css_class("alles-building");
            node.append(ProgressRing::new(16, None).widget());
            node.append(&label(t("Bijwerken…", "Updating…")));
        } else if st["state"] == "ready" {
            node.add_css_class("alles-ready");
            let ok = kit::icons::glyph("check", 14);
            ok.add_css_class("alles-idx-ok");
            node.append(&ok);
            node.append(&label(t("Index bijgewerkt", "Index up to date")));
            if live_on(&st) {
                node.append(&controls::label("·", "alles-idx-dot", false));
                let l = kit::icons::glyph("lightning", 14);
                l.add_css_class("alles-idx-live");
                node.append(&l);
                node.append(&label(t("Live bijwerken aan", "Live updates on")));
            }
            let n = node.clone();
            let update: Update = Rc::new(move |app: &Rc<App>| {
                let at = app.st.borrow().status.as_ref().and_then(|s| s["builtAt"].as_f64()).filter(|a| *a > 0.0);
                n.set_tooltip_text(at.map(built_at).as_deref());
            });
            *self.update_index.borrow_mut() = Some(update);
        } else if st["state"] == "error" {
            let b = gtk::Button::new();
            b.add_css_class("alles-idx");
            b.add_css_class("alles-idx-error");
            b.set_widget_name("alles-index-error");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&kit::icons::glyph("warning", 14));
            let l = label("");
            row.append(&l);
            b.set_child(Some(&row));
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.refresh_status(|| {});
                }
            });
            let update: Update = Rc::new(move |app: &Rc<App>| {
                let msg = app.st.borrow().status.as_ref().and_then(|s| s["message"].as_str().map(str::to_owned)).filter(|m| !m.is_empty());
                l.set_label(&msg.unwrap_or_else(|| t("Index niet beschikbaar", "Index unavailable").to_owned()));
            });
            *self.update_index.borrow_mut() = Some(update.clone());
            self.idx.append(&b);
            update(self);
            return;
        } else {
            node.append(&kit::icons::glyph("info", 14));
            node.append(&label(t("Nog geen index", "No index yet")));
        }
        self.idx.append(&node);
        let u = self.update_index.borrow().clone();
        if let Some(u) = u {
            u(self);
        }
    }

    fn apply_status(self: &Rc<Self>, st: Value) {
        if !st.is_object() {
            return;
        }
        let was_first = self.is_first_run();
        self.st.borrow_mut().status = Some(st);
        self.render_index_state();
        let first = self.is_first_run();
        if first {
            self.show_overlay(Some(Overlay::First));
        } else if was_first {
            self.run_query(false, false);
        } else if self.st.borrow().overlay == Some(Overlay::Welcome) {
            self.show_overlay(Some(Overlay::Welcome));
        }
        self.render_counts();
    }

    fn refresh_status(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let weak = Rc::downgrade(self);
        self.ctx.call("index.status", json!({}), move |r| {
            let Some(app) = weak.upgrade() else { return };
            match r {
                Ok(st) => app.apply_status(st),
                Err(e) => app.apply_status(json!({ "state": "error", "message": e.message, "files": 0, "folders": 0 })),
            }
            then();
        });
    }

    /// index.changed, at most once every 1.2 s (main.js throttle).
    fn index_changed(self: &Rc<Self>) {
        let now = Instant::now();
        let wait = Duration::from_millis(1200);
        let due = self.changed_last.get().map(|l| now.duration_since(l)).is_none_or(|d| d >= wait);
        let run = |app: &Rc<App>| {
            app.changed_last.set(Some(Instant::now()));
            if app.active_query() {
                app.run_query(true, true);
            } else {
                app.refresh_status(|| {});
            }
        };
        if due {
            run(self);
            return;
        }
        if self.changed_timer.borrow().is_some() {
            return;
        }
        let left = self.changed_last.get().map(|l| wait.saturating_sub(now.duration_since(l))).unwrap_or(wait);
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(left, move || {
            if let Some(app) = weak.upgrade() {
                app.changed_timer.borrow_mut().take();
                run(&app);
            }
        });
        *self.changed_timer.borrow_mut() = Some(id);
    }

    // ----------------------------------------------------------------------------------- test helpers

    /// main.js debugOpen: ?select=3 (or 1,4), ?open=options|tips|preview|menu|properties|trash|sort|filter,
    /// ?scrollTo=0..1, for screenshots and tests.
    fn debug_open(self: &Rc<Self>, p: &Params) {
        if p.select.is_none() && p.open.is_none() {
            return;
        }
        let p = p.clone();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(app) = weak.upgrade() else { return };
            if let Some(sel) = &p.select {
                let rows: Vec<u32> = sel.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if !rows.is_empty() {
                    app.table.select_rows(&rows);
                    app.st.borrow_mut().focus = rows.first().copied();
                }
            }
            let i = app.focus_index().unwrap_or(0);
            let item = app.model.entry(i);
            match p.open.as_deref() {
                Some("options") | Some("options-bottom") => app.open_options(),
                Some("tips") => app.set_pane(Some("tips")),
                Some("preview") => app.set_pane(Some("preview")),
                Some("properties") => {
                    if let Some(e) = item {
                        actions::show_properties(app.parent(), &e);
                    }
                }
                Some("trash") => app.trash_selection(),
                Some("menu") => {
                    if item.is_some() {
                        app.open_row_menu(i, 200.0, 48.0 + i as f64 * 32.0);
                    }
                }
                Some("sort") => {
                    let b = app.sort_btn.button.clone();
                    app.open_sort_menu(&b);
                }
                Some("filter") => app.filter.widget().emit_clicked(),
                _ => {}
            }
            if let Some(sc) = p.scroll_to.as_deref().and_then(|s| s.parse::<f64>().ok()).filter(|s| (0.0..=1.0).contains(s)) {
                let total = app.model.total();
                app.table.scroll_to_index((sc * total.saturating_sub(1) as f64).floor() as u32);
            }
        });
    }
}
