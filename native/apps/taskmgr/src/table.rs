//! Task Manager's grid (js/apps/taskmgr/table.js DataTable) on kit::table::Table: sortable and resizable
//! columns (widths, hidden columns and the sort remembered per table), optional groups ("Apps (8)"), expandable
//! rows (chevron, child rows; a search auto-expands parents of matching children), heat-map cells, the totals
//! above the column titles ("23%" over CPU), single or multiple selection, activation, Delete, type-ahead on the
//! first column, a context menu per row and the column menu on the header.
//!
//! The order is Task Manager's own (groups, parents with their children under them, sorted within): the table
//! computes it and gives every row its rank; the kit's ColumnView sorts by that rank (`__rank`, signed by the
//! direction), so the column headers keep GTK's sort marks and the rows keep their objects, selection and
//! scroll position while the data refreshes every second.
use kit::controls::{MenuItem, TopAction};
use kit::prelude::*;
use kit::table::{Align, Column, Selection, SortKey, Table, TableCell};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// A sort value.
#[derive(Clone, Debug, PartialEq)]
pub enum SortVal {
    Num(f64),
    Text(String),
}

type TextFn = Rc<dyn Fn(&Value) -> String>;
type SortFn = Rc<dyn Fn(&Value) -> SortVal>;
type NumFn = Rc<dyn Fn(&Value) -> f64>;
type SetupFn = Rc<dyn Fn() -> gtk::Widget>;
type BindFn = Rc<dyn Fn(&gtk::Widget, &Value)>;
type MenuFn = Rc<dyn Fn(&[String]) -> Option<(Vec<TopAction>, Vec<MenuItem>)>>;

/// How a column draws its cells.
#[derive(Clone)]
pub enum CellKind {
    /// Text from the row.
    Text(TextFn),
    /// The name cell: row fields "name", "icon" (an app icon; "" for the glyph), "glyph", "count" (" (3)"),
    /// "nameClass"; the chevron of expandable rows when the table is a tree. `px` is the icon size.
    Name { px: i32 },
    /// Any widget: `setup` once per cell, `bind(widget, row)` per row and change. Group rows bind too.
    Custom { setup: SetupFn, bind: BindFn },
}

/// A column (table.js column definition), built with `Col::text` / `Col::name` / `Col::custom`.
#[derive(Clone)]
pub struct Col {
    pub id: &'static str,
    pub label: String,
    pub width: i32,
    pub flex: bool,
    pub right: bool,
    /// Always shown (not in the column menu).
    pub required: bool,
    /// Hidden by default.
    pub hidden: bool,
    pub sortable: bool,
    /// The first click sorts descending (CPU, memory, ...).
    pub default_desc: bool,
    pub header_tip: String,
    /// A CSS class on every cell (taskmgr-mono, taskmgr-muted).
    pub class: &'static str,
    pub cell: CellKind,
    pub value: SortFn,
    pub heat: Option<NumFn>,
    pub tip: Option<TextFn>,
    /// The text type-ahead matches (the first column's).
    pub search: Option<TextFn>,
}

impl Col {
    fn base(id: &'static str, label: &str, cell: CellKind, value: SortFn) -> Col {
        Col {
            id,
            label: label.to_owned(),
            width: 100,
            flex: false,
            right: false,
            required: false,
            hidden: false,
            sortable: true,
            default_desc: false,
            header_tip: String::new(),
            class: "",
            cell,
            value,
            heat: None,
            tip: None,
            search: None,
        }
    }

    /// A text column; it sorts by its text unless `value` says otherwise.
    pub fn text(id: &'static str, label: &str, f: impl Fn(&Value) -> String + 'static) -> Col {
        let f: TextFn = Rc::new(f);
        let g = f.clone();
        Col::base(id, label, CellKind::Text(f), Rc::new(move |v| SortVal::Text(g(v).to_lowercase())))
    }

    /// The name column (see CellKind::Name); sorts and type-ahead by "name".
    pub fn name(id: &'static str, label: &str, px: i32) -> Col {
        let mut c = Col::base(id, label, CellKind::Name { px }, Rc::new(|v| SortVal::Text(crate::util::text(&v["name"]).to_lowercase())));
        c.search = Some(Rc::new(|v| crate::util::text(&v["name"])));
        c
    }

    /// A custom column.
    pub fn custom(id: &'static str, label: &str, setup: impl Fn() -> gtk::Widget + 'static, bind: impl Fn(&gtk::Widget, &Value) + 'static) -> Col {
        Col::base(id, label, CellKind::Custom { setup: Rc::new(setup), bind: Rc::new(bind) }, Rc::new(|_| SortVal::Num(0.0)))
    }

    pub fn width(mut self, px: i32) -> Col {
        self.width = px;
        self
    }
    pub fn flex(mut self) -> Col {
        self.flex = true;
        self
    }
    pub fn right(mut self) -> Col {
        self.right = true;
        self
    }
    pub fn required(mut self) -> Col {
        self.required = true;
        self
    }
    pub fn hidden(mut self) -> Col {
        self.hidden = true;
        self
    }
    pub fn unsortable(mut self) -> Col {
        self.sortable = false;
        self
    }
    pub fn default_desc(mut self) -> Col {
        self.default_desc = true;
        self
    }
    pub fn header_tip(mut self, tip: &str) -> Col {
        self.header_tip = tip.to_owned();
        self
    }
    pub fn class(mut self, class: &'static str) -> Col {
        self.class = class;
        self
    }
    /// Sort by this value.
    pub fn value(mut self, f: impl Fn(&Value) -> SortVal + 'static) -> Col {
        self.value = Rc::new(f);
        self
    }
    /// Sort by a number field.
    pub fn num(self, field: &'static str) -> Col {
        self.value(move |v| SortVal::Num(v[field].as_f64().unwrap_or(0.0)))
    }
    /// Heat-map shading: f(row) 0..1.
    pub fn heat(mut self, f: impl Fn(&Value) -> f64 + 'static) -> Col {
        self.heat = Some(Rc::new(f));
        self
    }
    pub fn tip(mut self, f: impl Fn(&Value) -> String + 'static) -> Col {
        self.tip = Some(Rc::new(f));
        self
    }
}

/// A row: its key, its value (what the cells show) and its child rows.
#[derive(Clone, Debug, Default)]
pub struct TRow {
    pub key: String,
    pub value: Value,
    pub children: Vec<TRow>,
}

impl TRow {
    pub fn new(key: impl Into<String>, value: Value) -> TRow {
        TRow { key: key.into(), value, children: Vec::new() }
    }
}

/// A group of rows with a heading ("" for none).
#[derive(Clone, Debug, Default)]
pub struct TGroup {
    pub id: String,
    pub label: String,
    pub rows: Vec<TRow>,
}

/// Table options.
pub struct Opts {
    /// Remembered widths, hidden columns and sort under "table.<key>".
    pub storage_key: &'static str,
    pub multi: bool,
    /// Rows can have children (the chevron in the name cell).
    pub tree: bool,
    /// (column, descending) when nothing is remembered.
    pub default_sort: Option<(&'static str, bool)>,
    /// Classes for a row (taskmgr-row-hung, ...), set on the row widget.
    pub row_class: Option<Rc<dyn Fn(&Value) -> Vec<&'static str>>>,
    /// Row height class: "taskmgr-rows-30", "-40", "-44".
    pub rows_class: &'static str,
    /// Group headings show their row count.
    pub group_counts: bool,
}

#[derive(Clone)]
struct Item {
    key: String,
    depth: u8,
    parent: Option<String>,
    has_kids: bool,
    group: bool,
}

struct Inner {
    table: Table,
    cols: Vec<Col>,
    opts: Opts,
    groups: RefCell<Vec<TGroup>>,
    filter: RefCell<Option<Rc<dyn Fn(&Value) -> bool>>>,
    expanded: RefCell<HashSet<String>>,
    auto_expanded: RefCell<HashSet<String>>,
    sort: RefCell<Option<(String, bool)>>,
    hidden: RefCell<HashSet<String>>,
    widths: RefCell<HashMap<String, i32>>,
    header_labels: RefCell<HashMap<String, gtk::Label>>,
    header_values: RefCell<HashMap<String, String>>,
    items: RefCell<Vec<Item>>,
    values: RefCell<HashMap<String, Value>>,
    on_selection: RefCell<Vec<Rc<dyn Fn(&[String])>>>,
    on_activate: RefCell<Option<Rc<dyn Fn(&str)>>>,
    on_delete: RefCell<Option<Rc<dyn Fn(&[String])>>>,
    menu: RefCell<Option<MenuFn>>,
    empty: gtk::Label,
    sorting: Cell<bool>,
    loading: Cell<bool>,
    typed: RefCell<String>,
    typed_timer: RefCell<Option<glib::SourceId>>,
}

/// The grid (a cheap handle).
#[derive(Clone)]
pub struct TmTable {
    inner: Rc<Inner>,
}

fn rank_of(v: &Value) -> SortKey {
    SortKey::Num(v["__rank"].as_f64().unwrap_or(0.0))
}

fn is_group(v: &Value) -> bool {
    v["__group"] == true
}

/// The TableCell a cell's widget is in, and its row id.
pub fn cell_row_id(w: &impl IsA<gtk::Widget>) -> Option<String> {
    let mut cur = Some(w.as_ref().clone());
    while let Some(c) = cur {
        if let Some(cell) = c.downcast_ref::<TableCell>() {
            return cell.row().map(|r| r.id());
        }
        cur = c.parent();
    }
    None
}

/// The ColumnView row widget a cell's widget is in.
fn row_widget(w: &gtk::Widget) -> Option<gtk::Widget> {
    let mut cur = w.parent();
    for _ in 0..6 {
        let c = cur?;
        if c.css_name() == "row" {
            return Some(c);
        }
        cur = c.parent();
    }
    None
}

const ROW_CLASSES: [&str; 6] = ["taskmgr-row-group", "taskmgr-row-child", "taskmgr-row-hung", "taskmgr-row-system", "taskmgr-row-efficient", "taskmgr-row-first-group"];

fn compare_vals(a: &SortVal, b: &SortVal) -> Ordering {
    match (a, b) {
        (SortVal::Num(x), SortVal::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (SortVal::Text(x), SortVal::Text(y)) => kit::format::compare_natural(x, y),
        (SortVal::Num(_), SortVal::Text(_)) => Ordering::Less,
        (SortVal::Text(_), SortVal::Num(_)) => Ordering::Greater,
    }
}

impl TmTable {
    pub fn new(cols: Vec<Col>, opts: Opts) -> TmTable {
        let slot: Rc<RefCell<Weak<Inner>>> = Rc::new(RefCell::new(Weak::new()));
        let row_owner = cols.iter().position(|c| c.required).unwrap_or(0);
        let mut kcols: Vec<Column> = Vec::new();
        for (idx, c) in cols.iter().enumerate() {
            let mut k = Column::new(c.id, &c.label).width(c.width).sortable(c.sortable).sort_by(rank_of);
            if c.flex {
                k = k.expand();
            }
            if c.right {
                k = k.align(Align::End);
            }
            let owner = idx == row_owner;
            let row_class = opts.row_class.clone();
            let tree = opts.tree;
            k = match &c.cell {
                CellKind::Text(f) => {
                    let f = f.clone();
                    let right = c.right;
                    k.custom(
                        move || {
                            let l = gtk::Label::new(None);
                            l.set_xalign(if right { 1.0 } else { 0.0 });
                            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            l.set_single_line_mode(true);
                            l.set_hexpand(true);
                            l.upcast()
                        },
                        move |w, v| {
                            if owner {
                                apply_row_classes(w, row_class.clone());
                            }
                            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                                let t = if is_group(v) { String::new() } else { f(v) };
                                if l.label() != t {
                                    l.set_label(&t);
                                }
                            }
                        },
                    )
                }
                CellKind::Name { px } => {
                    let px = *px;
                    let slot2 = slot.clone();
                    k.custom(
                        move || name_setup(tree, px, slot2.clone()),
                        move |w, v| {
                            if owner {
                                apply_row_classes(w, row_class.clone());
                            }
                            name_bind(w, v, tree, px);
                        },
                    )
                }
                CellKind::Custom { setup, bind } => {
                    let setup = setup.clone();
                    let bind = bind.clone();
                    k.custom(
                        move || setup(),
                        move |w, v| {
                            if owner {
                                apply_row_classes(w, row_class.clone());
                            }
                            bind(w, v);
                        },
                    )
                }
            };
            let heat = c.heat.clone();
            let class = c.class;
            if heat.is_some() || !class.is_empty() {
                k = k.cell_class(move |v| {
                    if is_group(v) {
                        return None;
                    }
                    match &heat {
                        Some(h) => Some(format!("taskmgr-heat-{}", crate::util::heat_bucket(h(v)))),
                        None => Some(class.to_owned()),
                    }
                });
            }
            if let Some(tip) = c.tip.clone() {
                k = k.tooltip(move |v| if is_group(v) { String::new() } else { tip(v) });
            }
            kcols.push(k);
        }
        // the order Task Manager computed, while no column is sorted
        kcols.push(Column::new("__order", "").sort_by(rank_of).resizable(false));
        let table = Table::new(kcols, "__key", if opts.multi { Selection::Multi } else { Selection::Single });
        table.view().add_css_class("taskmgr-table");
        if !opts.rows_class.is_empty() {
            table.view().add_css_class(opts.rows_class);
        }
        table.set_column_visible("__order", false);
        let empty = gtk::Label::new(None);
        empty.add_css_class("taskmgr-empty");
        empty.set_wrap(true);
        empty.set_justify(gtk::Justification::Center);
        table.set_empty_widget(Some(&empty));
        empty.set_valign(gtk::Align::Start);
        empty.set_margin_top(80);

        let saved = crate::util::load(&format!("table.{}", opts.storage_key));
        let hidden: HashSet<String> = match saved["hidden"].as_array() {
            Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            None => cols.iter().filter(|c| c.hidden).map(|c| c.id.to_owned()).collect(),
        };
        let widths: HashMap<String, i32> = saved["widths"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_i64().map(|w| (k.clone(), w.clamp(40, 900) as i32))).collect())
            .unwrap_or_default();
        let sort = if saved.get("sort").is_some() {
            saved["sort"]["id"].as_str().filter(|id| cols.iter().any(|c| c.id == *id && c.sortable)).map(|id| (id.to_owned(), saved["sort"]["desc"] == true))
        } else {
            opts.default_sort.map(|(id, d)| (id.to_owned(), d))
        };
        let me = TmTable {
            inner: Rc::new(Inner {
                table,
                cols,
                opts,
                groups: RefCell::new(Vec::new()),
                filter: RefCell::new(None),
                expanded: RefCell::new(HashSet::new()),
                auto_expanded: RefCell::new(HashSet::new()),
                sort: RefCell::new(sort),
                hidden: RefCell::new(hidden),
                widths: RefCell::new(widths),
                header_labels: RefCell::new(HashMap::new()),
                header_values: RefCell::new(HashMap::new()),
                items: RefCell::new(Vec::new()),
                values: RefCell::new(HashMap::new()),
                on_selection: RefCell::new(Vec::new()),
                on_activate: RefCell::new(None),
                on_delete: RefCell::new(None),
                menu: RefCell::new(None),
                empty,
                sorting: Cell::new(false),
                loading: Cell::new(true),
                typed: RefCell::new(String::new()),
                typed_timer: RefCell::new(None),
            }),
        };
        *slot.borrow_mut() = Rc::downgrade(&me.inner);
        me.setup_columns();
        me.setup_header();
        me.setup_signals();
        me.setup_keys();
        me.apply_sort_marks();
        me.inner.loading.set(false);
        me
    }

    fn of(inner: &Weak<Inner>) -> Option<TmTable> {
        inner.upgrade().map(|inner| TmTable { inner })
    }

    /// The widget to put in the layout.
    pub fn widget(&self) -> gtk::Widget {
        self.inner.table.widget().clone().upcast()
    }

    /// The ColumnView (widget names for the dev channel).
    pub fn view(&self) -> gtk::ColumnView {
        self.inner.table.view().clone()
    }

    // ------------------------------------------------------------------------------------------- setup

    fn setup_columns(&self) {
        for c in &self.inner.cols {
            let Some(col) = self.inner.table.column(c.id) else { continue };
            if let Some(w) = self.inner.widths.borrow().get(c.id) {
                col.set_fixed_width(*w);
            }
            col.set_visible(c.required || !self.inner.hidden.borrow().contains(c.id));
            let weak = Rc::downgrade(&self.inner);
            let id = c.id;
            col.connect_fixed_width_notify(move |col| {
                let Some(me) = TmTable::of(&weak) else { return };
                if me.inner.loading.get() {
                    return;
                }
                me.inner.widths.borrow_mut().insert(id.to_owned(), col.fixed_width());
                me.persist();
            });
        }
    }

    fn setup_header(&self) {
        let view = self.inner.table.view();
        let Some(header) = view.first_child().filter(|h| h.css_name() == "header") else { return };
        // the title buttons follow the columns in the order they were added
        let mut titles: Vec<gtk::Widget> = Vec::new();
        let mut c = header.first_child();
        while let Some(t) = c {
            titles.push(t.clone());
            c = t.next_sibling();
        }
        let mut labels = HashMap::new();
        for (i, col) in self.inner.cols.iter().enumerate() {
            let Some(title) = titles.get(i) else { break };
            if !col.header_tip.is_empty() {
                title.set_tooltip_text(Some(&col.header_tip));
            }
            if let Some(l) = crate::util::first_label(title) {
                if col.right {
                    l.set_xalign(1.0);
                    l.set_justify(gtk::Justification::Right);
                    l.set_hexpand(true);
                }
                labels.insert(col.id.to_owned(), l);
            }
        }
        *self.inner.header_labels.borrow_mut() = labels;
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let weak = Rc::downgrade(&self.inner);
        let hw = header.downgrade();
        click.connect_pressed(move |g, _, x, y| {
            let (Some(me), Some(h)) = (TmTable::of(&weak), hw.upgrade()) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            let mut items = vec![MenuItem::header(kit::t("Kolommen", "Columns"))];
            items.extend(me.column_items());
            items.push(MenuItem::separator());
            let w = Rc::downgrade(&me.inner);
            items.push(
                MenuItem::new(kit::t("Standaardkolommen herstellen", "Reset columns"), move || {
                    if let Some(me) = TmTable::of(&w) {
                        me.reset_columns();
                    }
                })
                .glyph("refresh"),
            );
            kit::controls::show_menu_at(&h, x, y, items);
        });
        header.add_controller(click);
    }

    fn setup_signals(&self) {
        let weak = Rc::downgrade(&self.inner);
        self.inner.table.connect_selection_changed(move |ids| {
            let Some(me) = TmTable::of(&weak) else { return };
            if ids.iter().any(|id| id.starts_with("g:")) {
                let keep: Vec<&str> = ids.iter().filter(|id| !id.starts_with("g:")).map(String::as_str).collect();
                me.inner.table.select_ids(&keep);
                return;
            }
            let hs: Vec<Rc<dyn Fn(&[String])>> = me.inner.on_selection.borrow().clone();
            for h in hs {
                h(ids);
            }
        });
        let weak = Rc::downgrade(&self.inner);
        self.inner.table.connect_activate(move |id| {
            let Some(me) = TmTable::of(&weak) else { return };
            if id.starts_with("g:") {
                return;
            }
            let f = me.inner.on_activate.borrow().clone();
            match f {
                Some(f) => f(id),
                None => {
                    if me.has_kids(id) {
                        me.toggle_expand(id, None);
                    }
                }
            }
        });
        let weak = Rc::downgrade(&self.inner);
        self.inner.table.connect_sort_changed(move |col, asc| {
            let Some(me) = TmTable::of(&weak) else { return };
            if me.inner.sorting.get() {
                return;
            }
            me.user_sorted(col, asc);
        });
        let weak = Rc::downgrade(&self.inner);
        self.inner.table.connect_context_menu(move |ids, _, _| {
            let me = TmTable::of(&weak)?;
            if ids.is_empty() || ids.iter().any(|id| id.starts_with("g:")) {
                return None;
            }
            let f = me.inner.menu.borrow().clone()?;
            f(ids)
        });
    }

    fn setup_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(me) = TmTable::of(&weak) else { return glib::Propagation::Proceed };
            let mods = state & (gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK);
            // keys typed into a control inside a cell (a switch, a button) belong to that control
            if let Some(focus) = me.inner.table.view().root().and_then(|r| r.focus())
                && (focus.is::<gtk::Switch>() || focus.is::<gtk::Button>() || focus.is::<gtk::Text>())
            {
                return glib::Propagation::Proceed;
            }
            let sel = me.selection();
            let cur = sel.first().cloned();
            match key {
                gdk::Key::Delete if mods.is_empty() => {
                    let f = me.inner.on_delete.borrow().clone();
                    if let Some(f) = f
                        && !sel.is_empty()
                    {
                        f(&sel);
                        return glib::Propagation::Stop;
                    }
                }
                gdk::Key::Right if mods.is_empty() && me.inner.opts.tree => {
                    if let Some(k) = cur.clone()
                        && me.has_kids(&k)
                    {
                        if !me.is_expanded(&k) {
                            me.toggle_expand(&k, Some(true));
                        } else if let Some(next) = me.next_row(&k) {
                            me.select(&next, true);
                        }
                        return glib::Propagation::Stop;
                    }
                }
                gdk::Key::Left if mods.is_empty() && me.inner.opts.tree => {
                    if let Some(k) = cur.clone() {
                        if me.has_kids(&k) && me.is_expanded(&k) {
                            me.toggle_expand(&k, Some(false));
                            return glib::Propagation::Stop;
                        }
                        let parent = me.inner.items.borrow().iter().find(|i| i.key == k).and_then(|i| i.parent.clone());
                        if let Some(p) = parent {
                            me.select(&p, true);
                            return glib::Propagation::Stop;
                        }
                    }
                }
                _ => {}
            }
            if mods.is_empty()
                && let Some(ch) = key.to_unicode()
                && !ch.is_control()
                && !ch.is_whitespace()
                && me.type_ahead(ch, cur.as_deref())
            {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.inner.table.view().add_controller(keys);
    }

    fn type_ahead(&self, ch: char, cur: Option<&str>) -> bool {
        if let Some(t) = self.inner.typed_timer.borrow_mut().take() {
            t.remove();
        }
        self.inner.typed.borrow_mut().extend(ch.to_lowercase());
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local_once(Duration::from_millis(800), move || {
            if let Some(me) = TmTable::of(&weak) {
                me.inner.typed_timer.borrow_mut().take();
                me.inner.typed.borrow_mut().clear();
            }
        });
        *self.inner.typed_timer.borrow_mut() = Some(id);
        let typed = self.inner.typed.borrow().clone();
        let Some(search) = self.inner.cols.iter().find_map(|c| c.search.clone()) else { return false };
        let order: Vec<String> = self.inner.table.ids().into_iter().filter(|id| !id.starts_with("g:")).collect();
        if order.is_empty() {
            return false;
        }
        let i = cur.and_then(|c| order.iter().position(|k| k == c));
        let start = match i {
            Some(i) if typed.chars().count() == 1 => i + 1,
            Some(i) => i,
            None => 0,
        };
        let items = self.inner.items.borrow().clone();
        let depth: HashMap<&str, u8> = items.iter().map(|it| (it.key.as_str(), it.depth)).collect();
        let values = self.inner.values.borrow();
        let n = order.len();
        for step in 0..n {
            let k = &order[(start + step) % n];
            if depth.get(k.as_str()).copied().unwrap_or(0) != 0 {
                continue;
            }
            if let Some(v) = values.get(k)
                && search(v).to_lowercase().starts_with(&typed)
            {
                self.select(k, true);
                return true;
            }
        }
        false
    }

    fn next_row(&self, key: &str) -> Option<String> {
        let ids = self.inner.table.ids();
        let i = ids.iter().position(|k| k == key)?;
        ids.get(i + 1).cloned()
    }

    // --------------------------------------------------------------------------------------- the rows

    /// Rows in groups (Processes: Apps, Background processes, System processes).
    pub fn set_groups(&self, groups: Vec<TGroup>) {
        *self.inner.groups.borrow_mut() = groups;
        self.render();
    }

    /// Rows without groups.
    pub fn set_rows(&self, rows: Vec<TRow>) {
        self.set_groups(vec![TGroup { id: "all".into(), label: String::new(), rows }]);
    }

    /// Show only rows (and children) for which `f(row)` holds (a parent of a match stays, expanded).
    pub fn set_filter(&self, f: Option<Rc<dyn Fn(&Value) -> bool>>) {
        *self.inner.filter.borrow_mut() = f;
        self.render();
    }

    /// The text over the empty table ("Processen laden…", "Geen processen gevonden voor …").
    pub fn set_empty_text(&self, text: &str) {
        if self.inner.empty.label() != text {
            self.inner.empty.set_label(text);
        }
    }

    /// The value of a row (as the page gave it).
    #[allow(dead_code)] // page API (Opslag's lists)
    pub fn value(&self, key: &str) -> Option<Value> {
        self.inner.values.borrow().get(key).cloned()
    }

    fn sorted<'a>(&self, rows: &'a [TRow]) -> Vec<&'a TRow> {
        let mut out: Vec<&TRow> = rows.iter().collect();
        let sort = self.inner.sort.borrow().clone();
        let Some((id, desc)) = sort else { return out };
        let Some(col) = self.inner.cols.iter().find(|c| c.id == id) else { return out };
        let first = &self.inner.cols[0];
        out.sort_by(|a, b| {
            let r = compare_vals(&(col.value)(&a.value), &(col.value)(&b.value));
            if r == Ordering::Equal {
                return compare_vals(&(first.value)(&a.value), &(first.value)(&b.value));
            }
            if desc { r.reverse() } else { r }
        });
        out
    }

    fn render(&self) {
        let filter = self.inner.filter.borrow().clone();
        let groups = self.inner.groups.borrow().clone();
        self.inner.auto_expanded.borrow_mut().clear();
        let expanded = self.inner.expanded.borrow().clone();
        let mut out: Vec<(Item, Value)> = Vec::new();
        let mut first_group = true;
        for g in &groups {
            let mut rows_out: Vec<(Item, Value)> = Vec::new();
            for row in self.sorted(&g.rows) {
                let kids = &row.children;
                let matches = filter.as_ref().is_none_or(|f| f(&row.value));
                let mut show: Vec<&TRow> = kids.iter().collect();
                if let Some(f) = &filter {
                    let km: Vec<&TRow> = kids.iter().filter(|k| f(&k.value)).collect();
                    if !matches && km.is_empty() {
                        continue;
                    }
                    if !matches {
                        show = km;
                        self.inner.auto_expanded.borrow_mut().insert(row.key.clone());
                    }
                }
                let has_kids = !kids.is_empty();
                let open = has_kids && (expanded.contains(&row.key) || self.inner.auto_expanded.borrow().contains(&row.key));
                let mut v = row.value.clone();
                v["__depth"] = json!(0);
                v["__hasKids"] = json!(has_kids);
                v["__open"] = json!(open);
                rows_out.push((Item { key: row.key.clone(), depth: 0, parent: None, has_kids, group: false }, v));
                if open {
                    let owned: Vec<TRow> = show.into_iter().cloned().collect();
                    for kid in self.sorted(&owned) {
                        let mut v = kid.value.clone();
                        v["__depth"] = json!(1);
                        v["__hasKids"] = json!(false);
                        v["__open"] = json!(false);
                        rows_out.push((Item { key: kid.key.clone(), depth: 1, parent: Some(row.key.clone()), has_kids: false, group: false }, v));
                    }
                }
            }
            let top = rows_out.iter().filter(|(i, _)| i.depth == 0).count();
            if !g.label.is_empty() && top > 0 {
                let label = if self.inner.opts.group_counts { format!("{} ({top})", g.label) } else { g.label.clone() };
                let key = format!("g:{}", g.id);
                out.push((
                    Item { key: key.clone(), depth: 0, parent: None, has_kids: false, group: true },
                    json!({ "__group": true, "__label": label, "__first": first_group }),
                ));
                first_group = false;
            }
            out.extend(rows_out);
        }
        // the ColumnView inverts its sorter for a descending column: the ranks are negated then
        let sign = if self.kit_sort_desc() { -1.0 } else { 1.0 };
        let mut values = HashMap::with_capacity(out.len());
        let mut rows = Vec::with_capacity(out.len());
        let mut items = Vec::with_capacity(out.len());
        for (rank, (item, mut v)) in out.into_iter().enumerate() {
            if !item.group {
                let mut orig = v.clone();
                if let Some(o) = orig.as_object_mut() {
                    o.remove("__depth");
                    o.remove("__hasKids");
                    o.remove("__open");
                }
                values.insert(item.key.clone(), orig);
            }
            v["__key"] = json!(item.key);
            v["__rank"] = json!(sign * rank as f64);
            rows.push(v);
            items.push(item);
        }
        *self.inner.items.borrow_mut() = items;
        *self.inner.values.borrow_mut() = values;
        self.inner.table.set_rows(rows);
    }

    /// Whether the kit's ColumnView sorts descending now (its primary column).
    fn kit_sort_desc(&self) -> bool {
        self.inner.table.sort().is_some_and(|(_, asc)| !asc)
    }

    // ---------------------------------------------------------------------------------------- sorting

    fn user_sorted(&self, col: Option<&str>, asc: bool) {
        let col = col.filter(|c| *c != "__order").map(str::to_owned);
        match col {
            None => *self.inner.sort.borrow_mut() = None,
            Some(id) => {
                let prev = self.inner.sort.borrow().clone();
                let def = self.inner.cols.iter().find(|c| c.id == id).map(|c| c.default_desc).unwrap_or(false);
                let mut desc = !asc;
                if prev.as_ref().map(|p| p.0.as_str()) != Some(id.as_str()) && def && asc {
                    desc = true;
                    self.inner.sorting.set(true);
                    self.inner.table.set_sort(Some(&id), false);
                    self.inner.sorting.set(false);
                }
                *self.inner.sort.borrow_mut() = Some((id, desc));
            }
        }
        self.persist();
        self.render();
    }

    fn apply_sort_marks(&self) {
        let sort = self.inner.sort.borrow().clone();
        self.inner.sorting.set(true);
        match sort {
            Some((id, desc)) => self.inner.table.set_sort(Some(&id), !desc),
            None => self.inner.table.set_sort(Some("__order"), true),
        }
        self.inner.sorting.set(false);
    }

    /// Sort by a column (a deep link's sort=cpu).
    pub fn sort_by(&self, id: &str, desc: bool) {
        if !self.inner.cols.iter().any(|c| c.id == id) {
            return;
        }
        *self.inner.sort.borrow_mut() = Some((id.to_owned(), desc));
        self.apply_sort_marks();
        self.persist();
        self.render();
    }

    // -------------------------------------------------------------------------------------- selection

    /// The selected keys (in the order shown).
    pub fn selection(&self) -> Vec<String> {
        self.inner.table.selected_ids().into_iter().filter(|id| !id.starts_with("g:")).collect()
    }

    /// Select one row (and scroll to it; `focus` also gives it the focus). False when it is not shown.
    pub fn select(&self, key: &str, focus: bool) -> bool {
        if !self.inner.table.ids().iter().any(|k| k == key) {
            return false;
        }
        self.inner.table.select_ids(&[key]);
        if focus {
            self.inner.table.scroll_to(key);
        }
        true
    }

    #[allow(dead_code)] // page API (Opslag's lists)
    pub fn clear_selection(&self) {
        self.inner.table.clear_selection();
    }

    pub fn connect_selection_changed(&self, f: impl Fn(&[String]) + 'static) {
        self.inner.on_selection.borrow_mut().push(Rc::new(f));
    }

    /// Double click / Enter on a row (without it, a row with children opens or closes).
    pub fn connect_activate(&self, f: impl Fn(&str) + 'static) {
        *self.inner.on_activate.borrow_mut() = Some(Rc::new(f));
    }

    /// The Delete key with a selection.
    pub fn connect_delete(&self, f: impl Fn(&[String]) + 'static) {
        *self.inner.on_delete.borrow_mut() = Some(Rc::new(f));
    }

    /// The context menu of the selected rows (right-click, Menu key, Shift+F10).
    pub fn connect_context_menu(&self, f: impl Fn(&[String]) -> Option<(Vec<TopAction>, Vec<MenuItem>)> + 'static) {
        *self.inner.menu.borrow_mut() = Some(Rc::new(f));
    }

    // ------------------------------------------------------------------------------------------ the tree

    pub fn has_kids(&self, key: &str) -> bool {
        self.inner.items.borrow().iter().any(|i| i.key == key && i.has_kids)
    }

    pub fn is_expanded(&self, key: &str) -> bool {
        self.inner.expanded.borrow().contains(key) || self.inner.auto_expanded.borrow().contains(key)
    }

    fn toggle_expand(&self, key: &str, open: Option<bool>) {
        let want = open.unwrap_or(!self.is_expanded(key));
        if want {
            self.inner.expanded.borrow_mut().insert(key.to_owned());
        } else {
            self.inner.expanded.borrow_mut().remove(key);
            self.inner.auto_expanded.borrow_mut().remove(key);
        }
        self.render();
    }

    /// Open or close a row's children.
    pub fn expand(&self, key: &str, open: bool) {
        self.toggle_expand(key, Some(open));
    }

    /// Open (or close) every row with children.
    pub fn expand_all(&self, open: bool) {
        if open {
            let keys: Vec<String> = self.inner.items.borrow().iter().filter(|i| i.has_kids).map(|i| i.key.clone()).collect();
            self.inner.expanded.borrow_mut().extend(keys);
        } else {
            self.inner.expanded.borrow_mut().clear();
        }
        self.render();
    }

    // ---------------------------------------------------------------------------------------- columns

    /// The values above the column titles ({"cpu": "23%"}); empty values show only the title.
    pub fn set_header_values(&self, values: &[(&str, String)]) {
        let mut map: HashMap<String, String> = values.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
        let color = fg();
        if !map.is_empty() {
            map.insert("__fg".into(), color.clone());
        }
        if *self.inner.header_values.borrow() == map {
            return;
        }
        let any = !map.is_empty();
        for c in &self.inner.cols {
            let Some(l) = self.inner.header_labels.borrow().get(c.id).cloned() else { continue };
            let title = glib::markup_escape_text(&c.label);
            if !any {
                l.set_use_markup(false);
                l.set_label(&c.label);
                continue;
            }
            let top = match map.get(c.id) {
                Some(v) => format!("<span font_size='11.25pt' weight='600' foreground='{color}'>{}</span>", glib::markup_escape_text(v)),
                None => "<span font_size='11.25pt'> </span>".to_owned(),
            };
            l.set_use_markup(true);
            l.set_label(&format!("{top}\n{title}"));
        }
        *self.inner.header_values.borrow_mut() = map;
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.inner.hidden.borrow().contains(id)
    }

    pub fn set_column_hidden(&self, id: &str, hidden: bool) {
        if hidden {
            self.inner.hidden.borrow_mut().insert(id.to_owned());
        } else {
            self.inner.hidden.borrow_mut().remove(id);
        }
        self.inner.table.set_column_visible(id, !hidden);
        self.persist();
    }

    /// Check items for the columns that can be hidden (the header menu, Processes' View > Columns).
    pub fn column_items(&self) -> Vec<MenuItem> {
        self.inner
            .cols
            .iter()
            .filter(|c| !c.required)
            .map(|c| {
                let id = c.id;
                let shown = !self.is_hidden(id);
                let weak = Rc::downgrade(&self.inner);
                MenuItem::new(&c.label, move || {
                    if let Some(me) = TmTable::of(&weak) {
                        me.set_column_hidden(id, shown);
                    }
                })
                .checked(shown)
                .name(&format!("taskmgr-col-{id}"))
            })
            .collect()
    }

    fn reset_columns(&self) {
        self.inner.loading.set(true);
        self.inner.widths.borrow_mut().clear();
        let defaults: HashSet<String> = self.inner.cols.iter().filter(|c| c.hidden).map(|c| c.id.to_owned()).collect();
        *self.inner.hidden.borrow_mut() = defaults.clone();
        for c in &self.inner.cols {
            if let Some(col) = self.inner.table.column(c.id) {
                col.set_fixed_width(c.width);
                col.set_visible(c.required || !defaults.contains(c.id));
            }
        }
        self.inner.loading.set(false);
        self.persist();
    }

    fn persist(&self) {
        if self.inner.loading.get() {
            return;
        }
        let widths: serde_json::Map<String, Value> = self.inner.widths.borrow().iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let mut hidden: Vec<String> = self.inner.hidden.borrow().iter().cloned().collect();
        hidden.sort();
        let sort = match self.inner.sort.borrow().as_ref() {
            Some((id, desc)) => json!({ "id": id, "desc": desc }),
            None => Value::Null,
        };
        crate::util::save(&format!("table.{}", self.inner.opts.storage_key), json!({ "widths": widths, "hidden": hidden, "sort": sort }));
    }
}

/// The text colour for the header values (markup cannot use CSS variables).
fn fg() -> String {
    let c = kit::theme::token("text-1");
    format!("#{:02x}{:02x}{:02x}", (c.red() * 255.0).round() as u8, (c.green() * 255.0).round() as u8, (c.blue() * 255.0).round() as u8)
}

type RowClassFn = Rc<dyn Fn(&Value) -> Vec<&'static str>>;

thread_local! {
    static ROW_PENDING: RefCell<Vec<(glib::WeakRef<gtk::Widget>, Option<RowClassFn>)>> = const { RefCell::new(Vec::new()) };
}

/// Style the row widget a cell is in (group headings, child rows, ...). Changing the ColumnView's row widget
/// while it binds its cells (inside the model's items-changed) crashes GTK 4.22, so this runs from an idle
/// callback, for the row the cell shows by then.
fn apply_row_classes(w: &gtk::Widget, row_class: Option<RowClassFn>) {
    let first = ROW_PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.push((w.downgrade(), row_class));
        p.len() == 1
    });
    if !first {
        return;
    }
    glib::idle_add_local_full(glib::Priority::DEFAULT, || {
        let pending: Vec<(glib::WeakRef<gtk::Widget>, Option<RowClassFn>)> = ROW_PENDING.with(|p| p.borrow_mut().drain(..).collect());
        for (wk, rc) in pending {
            let Some(w) = wk.upgrade() else { continue };
            let Some(cell) = w.parent().and_downcast::<TableCell>() else { continue };
            let Some(row_obj) = cell.row() else { continue };
            let Some(row) = row_widget(&w) else { continue };
            let v = row_obj.value();
            let extra: Vec<&'static str> = match &rc {
                Some(f) if !is_group(&v) => f(&v),
                _ => Vec::new(),
            };
            set_row_classes(&row, &v, &extra);
        }
        glib::ControlFlow::Break
    });
}

fn set_row_classes(row: &gtk::Widget, v: &Value, extra: &[&'static str]) {
    let group = is_group(v);
    let mut want: Vec<&'static str> = extra.to_vec();
    if group {
        want.push("taskmgr-row-group");
        if v["__first"] == true {
            want.push("taskmgr-row-first-group");
        }
    }
    if v["__depth"].as_i64().unwrap_or(0) > 0 {
        want.push("taskmgr-row-child");
    }
    for c in ROW_CLASSES {
        crate::util::set_class(row, c, want.contains(&c));
    }
}

// ---------------------------------------------------------------------------------------------- name cell

fn name_setup(tree: bool, px: i32, table: Rc<RefCell<Weak<Inner>>>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("taskmgr-name");
    if tree {
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slot.add_css_class("taskmgr-tree");
        slot.set_size_request(22, -1);
        let chev = gtk::Button::new();
        chev.add_css_class("taskmgr-chev");
        chev.set_child(Some(&kit::icons::glyph("chevron-right", 12)));
        chev.set_can_focus(false);
        chev.set_focus_on_click(false);
        chev.set_valign(gtk::Align::Center);
        chev.set_size_request(20, 20);
        chev.connect_clicked(move |b| {
            let Some(inner) = table.borrow().upgrade() else { return };
            let me = TmTable { inner };
            if let Some(id) = cell_row_id(b) {
                me.toggle_expand(&id, None);
            }
        });
        slot.append(&chev);
        root.append(&slot);
    }
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_hexpand(true);
    let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    icon.add_css_class("taskmgr-name-icon");
    icon.set_size_request(px, px);
    icon.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(None);
    name.add_css_class("taskmgr-name-text");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_single_line_mode(true);
    let count = gtk::Label::new(None);
    count.add_css_class("taskmgr-name-count");
    let group = gtk::Label::new(None);
    group.add_css_class("taskmgr-group-label");
    group.set_xalign(0.0);
    group.set_ellipsize(gtk::pango::EllipsizeMode::End);
    group.set_hexpand(true);
    group.set_valign(gtk::Align::End);
    group.set_visible(false);
    let texts = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    texts.append(&name);
    texts.append(&count);
    body.append(&icon);
    body.append(&texts);
    root.append(&body);
    root.append(&group);
    root.upcast()
}

fn name_bind(w: &gtk::Widget, v: &Value, tree: bool, px: i32) {
    let mut kids: Vec<gtk::Widget> = Vec::new();
    let mut c = w.first_child();
    while let Some(k) = c {
        kids.push(k.clone());
        c = k.next_sibling();
    }
    let (slot, body, group) = if tree {
        match (kids.first(), kids.get(1), kids.get(2)) {
            (Some(s), Some(b), Some(g)) => (Some(s.clone()), b.clone(), g.clone()),
            _ => return,
        }
    } else {
        match (kids.first(), kids.get(1)) {
            (Some(b), Some(g)) => (None, b.clone(), g.clone()),
            _ => return,
        }
    };
    let Some(group) = group.downcast_ref::<gtk::Label>() else { return };
    if is_group(v) {
        body.set_visible(false);
        if let Some(s) = &slot {
            s.set_visible(false);
        }
        group.set_visible(true);
        group.set_label(&crate::util::text(&v["__label"]));
        return;
    }
    group.set_visible(false);
    body.set_visible(true);
    let depth = v["__depth"].as_i64().unwrap_or(0);
    if let Some(s) = &slot {
        s.set_visible(depth == 0);
        if let Some(chev) = s.first_child() {
            let has = v["__hasKids"] == true;
            chev.set_opacity(if has { 1.0 } else { 0.0 });
            chev.set_can_target(has);
            if let Some(img) = chev.first_child().and_downcast::<gtk::Image>() {
                kit::icons::set_glyph(&img, if v["__open"] == true { "chevron-down" } else { "chevron-right" });
            }
        }
        body.set_margin_start(if depth > 0 { 34 - 8 } else { 0 });
    }
    let mut parts: Vec<gtk::Widget> = Vec::new();
    let mut c = body.first_child();
    while let Some(k) = c {
        parts.push(k.clone());
        c = k.next_sibling();
    }
    let texts = parts.get(1);
    let (Some(icon), Some(name), Some(count)) = (
        parts.first().and_then(|w| w.downcast_ref::<gtk::Box>().cloned()),
        texts.and_then(|t| t.first_child()).and_downcast::<gtk::Label>(),
        texts.and_then(|t| t.last_child()).and_downcast::<gtk::Label>(),
    ) else {
        return;
    };
    let app = crate::util::text(&v["icon"]);
    let glyph = v["glyph"].as_str().unwrap_or("window");
    let key = format!("{app}|{glyph}");
    if icon.widget_name() != key || icon.first_child().is_none() {
        icon.set_widget_name(&key);
        kit::controls::clear(&icon);
        icon.append(&crate::util::app_or_glyph(&app, glyph, px));
    }
    let text = crate::util::text(&v["name"]);
    if name.label() != text {
        name.set_label(&text);
    }
    for c in ["taskmgr-hang"] {
        crate::util::set_class(&name, c, v["nameClass"].as_str() == Some(c));
    }
    let cnt = crate::util::text(&v["count"]);
    if count.label() != cnt {
        count.set_label(&cnt);
    }
    count.set_visible(!cnt.is_empty());
}
