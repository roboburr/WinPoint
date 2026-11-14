//! A sortable table on gtk::ColumnView: columns (id, title, width, alignment, sortable, a text, icon + text or
//! custom renderer), rows as serde_json::Value keyed by an id field, client-side sorting by the column headers
//! (or the app's own order with `set_external_sort`), no / single / multiple selection, activation (double
//! click, Enter), a context menu per row (right-click, Menu key, Shift+F10) and ColumnView's keyboard
//! navigation. `set_rows` updates in place: rows keep their objects (so the selection and scroll position
//! stay), changed values redraw only the bound cells, and added or removed rows are spliced in runs, so a
//! once-a-second refresh (Task Manager) and tens of thousands of rows (Alles) stay fast.
//!
//! CSS: columnview.w11-table (rows 32 px; .w11-compact 24 px), cells hold a `w11-cell` widget with the class
//! `w11-col-<id>`, plus the class a column's `cell_class` returns for the row.
use crate::controls::menu::{self, MenuItem, TopAction};
use crate::icons;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

// ------------------------------------------------------------------------------------------------ the row

mod row_imp {
    use super::*;
    use glib::subclass::Signal;

    #[derive(Default)]
    pub struct Row {
        pub id: RefCell<String>,
        pub value: RefCell<Value>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Row {
        const NAME: &'static str = "W11KitTableRow";
        type Type = super::Row;
    }

    impl ObjectImpl for Row {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("changed").build()])
        }
    }
}

glib::wrapper! {
    /// A table row: its id and JSON value.
    pub struct Row(ObjectSubclass<row_imp::Row>);
}

impl Row {
    fn new(id: String, value: Value) -> Row {
        let r: Row = glib::Object::new();
        *r.imp().id.borrow_mut() = id;
        *r.imp().value.borrow_mut() = value;
        r
    }

    /// The row's id.
    pub fn id(&self) -> String {
        self.imp().id.borrow().clone()
    }

    /// The row's value.
    pub fn value(&self) -> Value {
        self.imp().value.borrow().clone()
    }

    fn set_value(&self, v: Value) -> bool {
        if *self.imp().value.borrow() == v {
            return false;
        }
        *self.imp().value.borrow_mut() = v;
        self.emit_by_name::<()>("changed", &[]);
        true
    }
}

// --------------------------------------------------------------------------------------------- the cell

mod cell_imp {
    use super::*;

    #[derive(Default)]
    pub struct Cell {
        pub row: RefCell<Option<super::Row>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub child: RefCell<Option<gtk::Widget>>,
        pub last_icon: RefCell<String>,
        pub last_class: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Cell {
        const NAME: &'static str = "W11KitTableCell";
        type Type = super::TableCell;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Cell {
        fn dispose(&self) {
            if let Some(c) = self.child.borrow_mut().take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Cell {}
}

glib::wrapper! {
    /// The widget in every table cell; `row()` is the row it shows.
    pub struct TableCell(ObjectSubclass<cell_imp::Cell>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TableCell {
    fn new(child: &gtk::Widget) -> TableCell {
        let c: TableCell = glib::Object::new();
        c.add_css_class("w11-cell");
        child.set_parent(&c);
        *c.imp().child.borrow_mut() = Some(child.clone());
        c
    }

    /// The row this cell shows now.
    pub fn row(&self) -> Option<Row> {
        self.imp().row.borrow().clone()
    }

    /// The content widget (what a custom renderer's `setup` made).
    pub fn content(&self) -> Option<gtk::Widget> {
        self.imp().child.borrow().clone()
    }
}

// ------------------------------------------------------------------------------------------------ columns

/// Cell alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

/// Where an icon + text column takes its icon from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    /// A glyph name in the icon field.
    Glyph,
    /// An app icon name / path in the icon field (icons::app_icon).
    App,
    /// A theme icon name or MIME type in the icon field (icons::file_icon).
    File,
    /// The whole row is a file item {icon, mime, isDir} (icons::file_item_icon, no thumbnail).
    Item,
}

/// A sort key for client-side sorting.
#[derive(Clone, Debug, PartialEq)]
pub enum SortKey {
    Num(f64),
    Text(String),
    None,
}

type TextFn = Rc<dyn Fn(&Value) -> String>;
type KeyFn = Rc<dyn Fn(&Value) -> SortKey>;
type ClassFn = Rc<dyn Fn(&Value) -> Option<String>>;
type SetupFn = Rc<dyn Fn() -> gtk::Widget>;
type BindFn = Rc<dyn Fn(&gtk::Widget, &Value)>;

#[derive(Clone)]
enum Renderer {
    Text,
    IconText { field: String, kind: IconKind, px: i32 },
    Custom { setup: SetupFn, bind: BindFn },
}

/// A table column, built with `Column::new(id, title)` and the chained setters.
#[derive(Clone)]
pub struct Column {
    pub id: String,
    pub title: String,
    /// Fixed width in px (0: by content).
    pub width: i32,
    pub expand: bool,
    pub align: Align,
    pub sortable: bool,
    pub resizable: bool,
    renderer: Renderer,
    text: Option<TextFn>,
    sort_key: Option<KeyFn>,
    cell_class: Option<ClassFn>,
    tooltip: Option<TextFn>,
}

impl Column {
    /// A text column showing value[id] (strings as they are, numbers plainly; `format` for more).
    pub fn new(id: &str, title: &str) -> Column {
        Column {
            id: id.to_owned(),
            title: title.to_owned(),
            width: 0,
            expand: false,
            align: Align::Start,
            sortable: true,
            resizable: true,
            renderer: Renderer::Text,
            text: None,
            sort_key: None,
            cell_class: None,
            tooltip: None,
        }
    }

    pub fn width(mut self, px: i32) -> Column {
        self.width = px;
        self
    }

    /// This column takes the spare width.
    pub fn expand(mut self) -> Column {
        self.expand = true;
        self
    }

    pub fn align(mut self, a: Align) -> Column {
        self.align = a;
        self
    }

    pub fn sortable(mut self, s: bool) -> Column {
        self.sortable = s;
        self
    }

    pub fn resizable(mut self, r: bool) -> Column {
        self.resizable = r;
        self
    }

    /// The cell text from the row (format::bytes, percent, dates, ...).
    pub fn format(mut self, f: impl Fn(&Value) -> String + 'static) -> Column {
        self.text = Some(Rc::new(f));
        self
    }

    /// The client-side sort key (default: value[id] as a number, else as text with natural order).
    pub fn sort_by(mut self, f: impl Fn(&Value) -> SortKey + 'static) -> Column {
        self.sort_key = Some(Rc::new(f));
        self
    }

    /// An icon before the text, from row[field] (or the whole row for IconKind::Item), `px` square.
    pub fn icon_text(mut self, field: &str, kind: IconKind, px: i32) -> Column {
        self.renderer = Renderer::IconText { field: field.to_owned(), kind, px };
        self
    }

    /// A custom cell: `setup` makes the widget once per cell, `bind` fills it for a row (again on changes).
    pub fn custom(mut self, setup: impl Fn() -> gtk::Widget + 'static, bind: impl Fn(&gtk::Widget, &Value) + 'static) -> Column {
        self.renderer = Renderer::Custom { setup: Rc::new(setup), bind: Rc::new(bind) };
        self
    }

    /// One extra CSS class on the cell per row (None: none), e.g. Task Manager's usage shading.
    pub fn cell_class(mut self, f: impl Fn(&Value) -> Option<String> + 'static) -> Column {
        self.cell_class = Some(Rc::new(f));
        self
    }

    /// A tooltip per cell.
    pub fn tooltip(mut self, f: impl Fn(&Value) -> String + 'static) -> Column {
        self.tooltip = Some(Rc::new(f));
        self
    }
}

/// How many rows can be selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    Single,
    Multi,
}

/// The text of a JSON value for a cell.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn default_key(v: &Value) -> SortKey {
    match v {
        Value::Number(n) => SortKey::Num(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => SortKey::Num(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => SortKey::Text(s.clone()),
        Value::Null => SortKey::None,
        other => SortKey::Text(other.to_string()),
    }
}

fn compare_keys(a: &SortKey, b: &SortKey) -> Ordering {
    match (a, b) {
        (SortKey::Num(x), SortKey::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (SortKey::Text(x), SortKey::Text(y)) => crate::format::compare_natural(x, y),
        (SortKey::None, SortKey::None) => Ordering::Equal,
        (SortKey::None, _) => Ordering::Greater,
        (_, SortKey::None) => Ordering::Less,
        (SortKey::Num(_), SortKey::Text(_)) => Ordering::Less,
        (SortKey::Text(_), SortKey::Num(_)) => Ordering::Greater,
    }
}

// ------------------------------------------------------------------------------------------------ the table

type MenuFn = Rc<dyn Fn(&[String], f64, f64) -> Option<(Vec<TopAction>, Vec<MenuItem>)>>;

struct Inner {
    root: glib::WeakRef<gtk::Overlay>,
    scroller: gtk::ScrolledWindow,
    view: gtk::ColumnView,
    store: gio::ListStore,
    sort_model: gtk::SortListModel,
    selection: gtk::SelectionModel,
    mode: Selection,
    id_key: String,
    rows: RefCell<HashMap<String, Row>>,
    columns: RefCell<Vec<(Column, gtk::ColumnViewColumn, Option<gtk::CustomSorter>)>>,
    external: Cell<bool>,
    empty: RefCell<Option<gtk::Widget>>,
    on_activate: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    on_selection: RefCell<Vec<Rc<dyn Fn(&[String])>>>,
    on_sort: RefCell<Vec<Rc<dyn Fn(Option<&str>, bool)>>>,
    menu: RefCell<Option<MenuFn>>,
}

/// The table. `widget()` is a scrolled overlay to put in the layout.
#[derive(Clone)]
pub struct Table {
    root: gtk::Overlay,
    inner: Rc<Inner>,
}

impl Table {
    /// Columns, the row field that identifies a row ("id", "pid", "path"), the selection mode.
    pub fn new(columns: Vec<Column>, id_key: &str, mode: Selection) -> Table {
        let store = gio::ListStore::new::<Row>();
        let view = gtk::ColumnView::new(None::<gtk::SelectionModel>);
        view.add_css_class("w11-table");
        view.set_reorderable(true);
        view.set_show_row_separators(false);
        view.set_show_column_separators(false);
        let sort_model = gtk::SortListModel::new(Some(store.clone()), view.sorter());
        let selection: gtk::SelectionModel = match mode {
            Selection::None => gtk::NoSelection::new(Some(sort_model.clone())).upcast(),
            Selection::Single => {
                let s = gtk::SingleSelection::new(Some(sort_model.clone()));
                s.set_autoselect(false);
                s.set_can_unselect(true);
                s.upcast()
            }
            Selection::Multi => gtk::MultiSelection::new(Some(sort_model.clone())).upcast(),
        };
        view.set_model(Some(&selection));
        view.set_enable_rubberband(mode == Selection::Multi);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&view));
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        let root = gtk::Overlay::new();
        root.set_child(Some(&scroller));
        let inner = Rc::new(Inner {
            root: root.downgrade(),
            scroller,
            view: view.clone(),
            store,
            sort_model,
            selection: selection.clone(),
            mode,
            id_key: id_key.to_owned(),
            rows: RefCell::new(HashMap::new()),
            columns: RefCell::new(Vec::new()),
            external: Cell::new(false),
            empty: RefCell::new(None),
            on_activate: RefCell::new(Vec::new()),
            on_selection: RefCell::new(Vec::new()),
            on_sort: RefCell::new(Vec::new()),
            menu: RefCell::new(None),
        });
        crate::controls::own(&root, &inner);
        let me = Table { root, inner };
        for c in columns {
            me.add_column(c);
        }
        let weak = Rc::downgrade(&me.inner);
        view.connect_activate(move |_, pos| {
            let Some(me) = weak.upgrade().and_then(Table::of) else { return };
            if let Some(id) = me.id_at(pos) {
                let hs: Vec<Rc<dyn Fn(&str)>> = me.inner.on_activate.borrow().clone();
                for h in hs {
                    h(&id);
                }
            }
        });
        let weak = Rc::downgrade(&me.inner);
        selection.connect_selection_changed(move |_, _, _| {
            let Some(me) = weak.upgrade().and_then(Table::of) else { return };
            let ids = me.selected_ids();
            let hs: Vec<Rc<dyn Fn(&[String])>> = me.inner.on_selection.borrow().clone();
            for h in hs {
                h(&ids);
            }
        });
        if let Some(sorter) = view.sorter() {
            let weak = Rc::downgrade(&me.inner);
            sorter.connect_changed(move |_, _| {
                let Some(me) = weak.upgrade().and_then(Table::of) else { return };
                let (col, asc) = match me.sort() {
                    Some((c, a)) => (Some(c), a),
                    None => (None, true),
                };
                let hs: Vec<Rc<dyn Fn(Option<&str>, bool)>> = me.inner.on_sort.borrow().clone();
                for h in hs {
                    h(col.as_deref(), asc);
                }
            });
        }
        me.install_menu();
        me
    }

    fn add_column(&self, c: Column) {
        let factory = gtk::SignalListItemFactory::new();
        let col = c.clone();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let content: gtk::Widget = match &col.renderer {
                Renderer::Text => text_label(col.align).upcast(),
                Renderer::IconText { .. } => {
                    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    slot.set_valign(gtk::Align::Center);
                    b.append(&slot);
                    let l = text_label(col.align);
                    l.set_hexpand(true);
                    b.append(&l);
                    b.upcast()
                }
                Renderer::Custom { setup, .. } => setup(),
            };
            let cell = TableCell::new(&content);
            cell.add_css_class(&format!("w11-col-{}", col.id));
            item.set_child(Some(&cell));
        });
        let col = c.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let (Some(row), Some(cell)) = (item.item().and_downcast::<Row>(), item.child().and_downcast::<TableCell>()) else {
                return;
            };
            if let Some(h) = cell.imp().handler.borrow_mut().take()
                && let Some(old) = cell.imp().row.borrow().as_ref()
            {
                old.disconnect(h);
            }
            *cell.imp().row.borrow_mut() = Some(row.clone());
            render(&col, &cell, &row.imp().value.borrow());
            let col2 = col.clone();
            let weak = cell.downgrade();
            let h = row.connect_local("changed", false, move |args| {
                if let (Some(cell), Ok(row)) = (weak.upgrade(), args[0].get::<Row>()) {
                    render(&col2, &cell, &row.imp().value.borrow());
                }
                None
            });
            *cell.imp().handler.borrow_mut() = Some(h);
        });
        factory.connect_unbind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let Some(cell) = item.child().and_downcast::<TableCell>() else { return };
            if let Some(h) = cell.imp().handler.borrow_mut().take()
                && let Some(old) = cell.imp().row.borrow().as_ref()
            {
                old.disconnect(h);
            }
            *cell.imp().row.borrow_mut() = None;
        });
        let column = gtk::ColumnViewColumn::new(Some(&c.title), Some(factory));
        column.set_resizable(c.resizable);
        column.set_expand(c.expand);
        if c.width > 0 {
            column.set_fixed_width(c.width);
        }
        column.set_id(Some(&c.id));
        let sorter = if c.sortable {
            let key = c.sort_key.clone();
            let id = c.id.clone();
            let s = gtk::CustomSorter::new(move |a, b| {
                let (Some(a), Some(b)) = (a.downcast_ref::<Row>(), b.downcast_ref::<Row>()) else { return gtk::Ordering::Equal };
                let va = a.imp().value.borrow();
                let vb = b.imp().value.borrow();
                let (ka, kb) = match &key {
                    Some(f) => (f(&va), f(&vb)),
                    None => (default_key(&va[id.as_str()]), default_key(&vb[id.as_str()])),
                };
                compare_keys(&ka, &kb).into()
            });
            column.set_sorter(Some(&s));
            Some(s)
        } else {
            None
        };
        self.inner.view.append_column(&column);
        self.inner.columns.borrow_mut().push((c, column, sorter));
    }

    /// The table to put in the layout (a scrolled window in an overlay for the empty state).
    fn of(inner: Rc<Inner>) -> Option<Table> {
        let root = inner.root.upgrade()?;
        Some(Table { root, inner })
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.root
    }

    /// The column view itself.
    pub fn view(&self) -> &gtk::ColumnView {
        &self.inner.view
    }

    /// The scrolled window.
    pub fn scroller(&self) -> &gtk::ScrolledWindow {
        &self.inner.scroller
    }

    fn id_of(&self, v: &Value) -> String {
        value_text(&v[self.inner.id_key.as_str()])
    }

    fn id_at(&self, pos: u32) -> Option<String> {
        self.inner.selection.item(pos).and_downcast::<Row>().map(|r| r.id())
    }

    /// Replace the rows, in place: same ids keep their row objects (selection, scroll), changed values redraw
    /// their cells, new and gone rows are spliced in runs. Rows without an id are skipped.
    pub fn set_rows(&self, rows: Vec<Value>) {
        let mut new_rows: Vec<Row> = Vec::with_capacity(rows.len());
        let mut seen: HashSet<String> = HashSet::with_capacity(rows.len());
        let mut changed = false;
        {
            let mut map = self.inner.rows.borrow_mut();
            for v in rows {
                let id = self.id_of(&v);
                if id.is_empty() || !seen.insert(id.clone()) {
                    continue;
                }
                match map.get(&id) {
                    Some(r) => {
                        changed |= r.set_value(v);
                        new_rows.push(r.clone());
                    }
                    None => {
                        let r = Row::new(id.clone(), v);
                        map.insert(id, r.clone());
                        new_rows.push(r);
                    }
                }
            }
            map.retain(|id, _| seen.contains(id));
        }
        let store = &self.inner.store;
        // 1. remove the rows that are gone, in runs from the end
        let n = store.n_items();
        let mut i = n;
        while i > 0 {
            i -= 1;
            let gone = |k: u32| store.item(k).and_downcast::<Row>().is_none_or(|r| !seen.contains(&*r.imp().id.borrow()));
            if gone(i) {
                let end = i + 1;
                while i > 0 && gone(i - 1) {
                    i -= 1;
                }
                store.splice(i, end - i, &[] as &[Row]);
            }
        }
        // 2. insert the new rows where they belong, in runs; a different order of the kept rows is a full
        //    replace, unless the table sorts by itself (then the store's order does not show)
        let kept: Vec<String> = (0..store.n_items())
            .filter_map(|k| store.item(k).and_downcast::<Row>().map(|r| r.id()))
            .collect();
        let kept_set: HashSet<String> = kept.iter().cloned().collect();
        let new_kept: Vec<String> = new_rows.iter().map(|r| r.id()).filter(|id| kept_set.contains(id)).collect();
        let same_order = new_kept == kept;
        let client_sorted = !self.inner.external.get() && self.sort().is_some();
        if same_order {
            let mut pos = 0u32;
            let mut pending: Vec<Row> = Vec::new();
            let mut insert_at = 0u32;
            for r in &new_rows {
                let id = r.imp().id.borrow().clone();
                if kept_set.contains(&id) {
                    if !pending.is_empty() {
                        store.splice(insert_at, 0, &pending);
                        pos += pending.len() as u32;
                        pending.clear();
                    }
                    pos += 1;
                    insert_at = pos;
                } else {
                    if pending.is_empty() {
                        insert_at = pos;
                    }
                    pending.push(r.clone());
                }
            }
            if !pending.is_empty() {
                store.splice(insert_at, 0, &pending);
            }
        } else if client_sorted {
            let added: Vec<Row> = new_rows.iter().filter(|r| !kept_set.contains(&*r.imp().id.borrow())).cloned().collect();
            if !added.is_empty() {
                store.splice(store.n_items(), 0, &added);
            }
        } else {
            store.splice(0, store.n_items(), &new_rows);
        }
        if changed && client_sorted {
            self.resort();
        }
        self.update_empty();
    }

    /// Update one row in place (added when new).
    pub fn update_row(&self, v: Value) {
        let id = self.id_of(&v);
        if id.is_empty() {
            return;
        }
        let existing = self.inner.rows.borrow().get(&id).cloned();
        match existing {
            Some(r) => {
                if r.set_value(v) && !self.inner.external.get() && self.sort().is_some() {
                    self.resort();
                }
            }
            None => {
                let r = Row::new(id.clone(), v);
                self.inner.rows.borrow_mut().insert(id, r.clone());
                self.inner.store.append(&r);
                self.update_empty();
            }
        }
    }

    /// Remove one row.
    pub fn remove_row(&self, id: &str) {
        if self.inner.rows.borrow_mut().remove(id).is_none() {
            return;
        }
        let store = &self.inner.store;
        for k in 0..store.n_items() {
            if store.item(k).and_downcast::<Row>().is_some_and(|r| r.id() == id) {
                store.remove(k);
                break;
            }
        }
        self.update_empty();
    }

    fn resort(&self) {
        if let Some((col, _)) = self.sort() {
            for (c, _, s) in self.inner.columns.borrow().iter() {
                if c.id == col
                    && let Some(s) = s
                {
                    s.changed(gtk::SorterChange::Different);
                }
            }
        }
    }

    /// A row's value by id.
    pub fn row(&self, id: &str) -> Option<Value> {
        self.inner.rows.borrow().get(id).map(|r| r.value())
    }

    /// The rows in the order shown.
    pub fn rows(&self) -> Vec<Value> {
        let m = &self.inner.selection;
        (0..m.n_items()).filter_map(|i| m.item(i).and_downcast::<Row>().map(|r| r.value())).collect()
    }

    /// The ids in the order shown.
    pub fn ids(&self) -> Vec<String> {
        let m = &self.inner.selection;
        (0..m.n_items()).filter_map(|i| m.item(i).and_downcast::<Row>().map(|r| r.id())).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.store.n_items() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // -------------------------------------------------------------------------------------- selection

    /// The selected ids, in the order shown.
    pub fn selected_ids(&self) -> Vec<String> {
        let sel = self.inner.selection.selection();
        let n = sel.size();
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            if let Some(id) = self.id_at(sel.nth(i as u32)) {
                out.push(id);
            }
        }
        out
    }

    /// Select exactly these ids (others unselected).
    pub fn select_ids(&self, ids: &[&str]) {
        let want: HashSet<&str> = ids.iter().copied().collect();
        let m = &self.inner.selection;
        let n = m.n_items();
        match self.inner.mode {
            Selection::None => {}
            Selection::Single => {
                if let Some(s) = m.downcast_ref::<gtk::SingleSelection>() {
                    let pos = (0..n).find(|&i| m.item(i).and_downcast::<Row>().is_some_and(|r| want.contains(r.imp().id.borrow().as_str())));
                    s.set_selected(pos.unwrap_or(gtk::INVALID_LIST_POSITION));
                }
            }
            Selection::Multi => {
                let selected = gtk::Bitset::new_empty();
                for i in 0..n {
                    if m.item(i).and_downcast::<Row>().is_some_and(|r| want.contains(r.imp().id.borrow().as_str())) {
                        selected.add(i);
                    }
                }
                let mask = gtk::Bitset::new_range(0, n);
                m.set_selection(&selected, &mask);
            }
        }
    }

    /// Select nothing.
    pub fn clear_selection(&self) {
        self.inner.selection.unselect_all();
    }

    /// Select every row (multiple selection).
    pub fn select_all(&self) {
        self.inner.selection.select_all();
    }

    /// Run `f(selected ids)` when the selection changes.
    pub fn connect_selection_changed(&self, f: impl Fn(&[String]) + 'static) {
        self.inner.on_selection.borrow_mut().push(Rc::new(f));
    }

    /// Run `f(id)` when a row is activated (double click, Enter).
    pub fn connect_activate(&self, f: impl Fn(&str) + 'static) {
        self.inner.on_activate.borrow_mut().push(Rc::new(f));
    }

    /// The context menu: `f(selected ids, x, y)` builds it (ids empty: on the empty area). A right-click on a
    /// row that is not selected selects it first.
    pub fn connect_context_menu(&self, f: impl Fn(&[String], f64, f64) -> Option<(Vec<TopAction>, Vec<MenuItem>)> + 'static) {
        *self.inner.menu.borrow_mut() = Some(Rc::new(f));
    }

    fn install_menu(&self) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let weak = Rc::downgrade(&self.inner);
        click.connect_pressed(move |g, _, x, y| {
            let Some(me) = weak.upgrade().and_then(Table::of) else { return };
            let Some(f) = me.inner.menu.borrow().clone() else { return };
            let id = me.row_at(x, y);
            if let Some(id) = &id
                && !me.selected_ids().contains(id)
            {
                me.select_ids(&[id.as_str()]);
            }
            let ids = if id.is_some() { me.selected_ids() } else { Vec::new() };
            if id.is_none() {
                me.clear_selection();
            }
            if let Some((top, items)) = f(&ids, x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                menu::show_menu_with_top(&me.inner.view, x, y, top, items);
            }
        });
        self.inner.view.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, state| {
            let menu_key = key == gdk::Key::Menu || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK));
            if !menu_key {
                return glib::Propagation::Proceed;
            }
            let Some(me) = weak.upgrade().and_then(Table::of) else { return glib::Propagation::Proceed };
            let Some(f) = me.inner.menu.borrow().clone() else { return glib::Propagation::Proceed };
            let ids = me.selected_ids();
            if let Some((top, items)) = f(&ids, 24.0, 40.0) {
                menu::show_menu_with_top(&me.inner.view, 24.0, 40.0, top, items);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.inner.view.add_controller(keys);
    }

    /// The id of the row under (x, y) in the view's coordinates.
    pub fn row_at(&self, x: f64, y: f64) -> Option<String> {
        let mut w = self.inner.view.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(cur) = w {
            if let Some(cell) = cur.downcast_ref::<TableCell>() {
                return cell.row().map(|r| r.id());
            }
            if cur == *self.inner.view.upcast_ref::<gtk::Widget>() {
                break;
            }
            w = cur.parent();
        }
        None
    }

    /// Scroll a row into view and give it the focus.
    pub fn scroll_to(&self, id: &str) {
        let m = &self.inner.selection;
        if let Some(pos) = (0..m.n_items()).find(|&i| m.item(i).and_downcast::<Row>().is_some_and(|r| r.id() == id)) {
            self.inner.view.scroll_to(pos, None::<&gtk::ColumnViewColumn>, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    // ------------------------------------------------------------------------------------------ sorting

    /// The sort in effect: (column id, ascending).
    pub fn sort(&self) -> Option<(String, bool)> {
        let sorter = self.inner.view.sorter().and_downcast::<gtk::ColumnViewSorter>()?;
        let col = sorter.primary_sort_column()?;
        let id = col.id()?.to_string();
        Some((id, sorter.primary_sort_order() == gtk::SortType::Ascending))
    }

    /// Sort by a column (None: unsorted).
    pub fn set_sort(&self, column: Option<&str>, ascending: bool) {
        let col = column.and_then(|id| self.inner.columns.borrow().iter().find(|(c, _, _)| c.id == id).map(|(_, col, _)| col.clone()));
        self.inner.view.sort_by_column(col.as_ref(), if ascending { gtk::SortType::Ascending } else { gtk::SortType::Descending });
    }

    /// Run `f(column, ascending)` when the sort changes (header click or `set_sort`).
    pub fn connect_sort_changed(&self, f: impl Fn(Option<&str>, bool) + 'static) {
        self.inner.on_sort.borrow_mut().push(Rc::new(f));
    }

    /// The app orders the rows itself (a backend that sorts): the headers still show and change the sort
    /// (`connect_sort_changed`), the table keeps the order of `set_rows`.
    pub fn set_external_sort(&self, external: bool) {
        self.inner.external.set(external);
        let sorter = if external { None } else { self.inner.view.sorter() };
        self.inner.sort_model.set_sorter(sorter.as_ref());
    }

    // ------------------------------------------------------------------------------------------- columns

    pub fn set_column_visible(&self, id: &str, visible: bool) {
        if let Some((_, col, _)) = self.inner.columns.borrow().iter().find(|(c, _, _)| c.id == id) {
            col.set_visible(visible);
        }
    }

    pub fn set_column_title(&self, id: &str, title: &str) {
        if let Some((_, col, _)) = self.inner.columns.borrow().iter().find(|(c, _, _)| c.id == id) {
            col.set_title(Some(title));
        }
    }

    /// The GTK column of an id (widths, visibility, ...).
    pub fn column(&self, id: &str) -> Option<gtk::ColumnViewColumn> {
        self.inner.columns.borrow().iter().find(|(c, _, _)| c.id == id).map(|(_, col, _)| col.clone())
    }

    /// 24 px rows instead of 32.
    pub fn set_compact(&self, compact: bool) {
        if compact {
            self.inner.view.add_css_class("w11-compact");
        } else {
            self.inner.view.remove_css_class("w11-compact");
        }
    }

    /// A widget shown over the table while it has no rows (an empty state); None removes it.
    pub fn set_empty_widget(&self, w: Option<&impl IsA<gtk::Widget>>) {
        if let Some(old) = self.inner.empty.borrow_mut().take() {
            self.root.remove_overlay(&old);
        }
        if let Some(w) = w {
            let w = w.as_ref().clone();
            w.set_halign(gtk::Align::Center);
            w.set_valign(gtk::Align::Center);
            self.root.add_overlay(&w);
            *self.inner.empty.borrow_mut() = Some(w);
        }
        self.update_empty();
    }

    fn update_empty(&self) {
        if let Some(w) = self.inner.empty.borrow().as_ref() {
            w.set_visible(self.inner.store.n_items() == 0);
        }
    }
}

fn text_label(align: Align) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(match align {
        Align::Start => 0.0,
        Align::Center => 0.5,
        Align::End => 1.0,
    });
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_single_line_mode(true);
    l.set_hexpand(true);
    l
}

fn render(col: &Column, cell: &TableCell, v: &Value) {
    let Some(content) = cell.content() else { return };
    let text = || match &col.text {
        Some(f) => f(v),
        None => value_text(&v[col.id.as_str()]),
    };
    match &col.renderer {
        Renderer::Text => {
            if let Some(l) = content.downcast_ref::<gtk::Label>() {
                let t = text();
                if l.label() != t {
                    l.set_label(&t);
                }
            }
        }
        Renderer::IconText { field, kind, px } => {
            let Some(b) = content.downcast_ref::<gtk::Box>() else { return };
            let (Some(slot), Some(label)) = (b.first_child().and_downcast::<gtk::Box>(), b.last_child().and_downcast::<gtk::Label>()) else {
                return;
            };
            let key = match kind {
                IconKind::Item => format!("{}|{}|{}", value_text(&v["icon"]), value_text(&v["mime"]), v["isDir"] == true),
                _ => value_text(&v[field.as_str()]),
            };
            if *cell.imp().last_icon.borrow() != key || slot.first_child().is_none() {
                *cell.imp().last_icon.borrow_mut() = key.clone();
                crate::controls::clear(&slot);
                let img = match kind {
                    IconKind::Glyph => icons::glyph(if key.is_empty() { "file" } else { &key }, *px),
                    IconKind::App => icons::app_icon(&key, *px),
                    IconKind::File => icons::file_icon(&key, *px),
                    IconKind::Item => icons::file_item_icon(v, *px, false),
                };
                slot.append(&img);
            }
            let t = text();
            if label.label() != t {
                label.set_label(&t);
            }
        }
        Renderer::Custom { bind, .. } => bind(&content, v),
    }
    if let Some(f) = &col.cell_class {
        let want = f(v);
        let mut last = cell.imp().last_class.borrow_mut();
        if *last != want {
            if let Some(old) = last.take() {
                cell.remove_css_class(&old);
            }
            if let Some(c) = &want {
                cell.add_css_class(c);
            }
            *last = want;
        }
    }
    if let Some(f) = &col.tooltip {
        let t = f(v);
        cell.set_tooltip_text(if t.is_empty() { None } else { Some(&t) });
    }
}
