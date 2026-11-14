//! The result table (table.js, the Win11 Explorer "Details" look) on gtk::ColumnView over the lazy model:
//! Naam (icon + name), Pad, Grootte, Gewijzigd op; resizable columns whose widths are remembered, headers that
//! sort through the backend (Grootte and Gewijzigd op start descending), multiple selection, matches in
//! semibold, placeholders for rows whose page is still loading, and a tooltip on cut-off names and paths.
use crate::highlight::{self, Highlight};
use crate::model::{Entry, Item, Results};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// The columns (table.js COLUMNS): id, default width, smallest width, starts descending.
pub const COLUMNS: [(&str, i32, i32, bool); 4] =
    [("name", 340, 150, false), ("path", 360, 120, false), ("size", 100, 70, true), ("modified", 148, 110, true)];

/// Whether a sort key starts descending when chosen (Grootte, Gewijzigd op).
pub fn first_desc(by: &str) -> bool {
    COLUMNS.iter().any(|(id, _, _, d)| *id == by && *d)
}

// ----------------------------------------------------------------------------------------------- the cell

mod cell_imp {
    use super::*;

    #[derive(Default)]
    pub struct AllesCell {
        pub col: Cell<usize>,
        pub item: RefCell<Option<Item>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub child: RefCell<Option<gtk::Widget>>,
        pub icon: RefCell<Option<gtk::Image>>,
        pub label: RefCell<Option<gtk::Label>>,
        pub bar: RefCell<Option<gtk::Widget>>,
        pub icon_key: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AllesCell {
        const NAME: &'static str = "AllesCell";
        type Type = super::AllesCell;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for AllesCell {
        fn dispose(&self) {
            if let Some(c) = self.child.borrow_mut().take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for AllesCell {}
}

glib::wrapper! {
    /// The widget in every result cell; `item()` is the row it shows.
    pub struct AllesCell(ObjectSubclass<cell_imp::AllesCell>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AllesCell {
    pub fn item(&self) -> Option<Item> {
        self.imp().item.borrow().clone()
    }
}

/// A placeholder bar `parts` of `of` wide (the pulsing bars of a row whose page is loading).
fn placeholder(parts: i32, of: i32) -> gtk::Widget {
    let g = gtk::Grid::new();
    g.set_column_homogeneous(true);
    g.set_hexpand(true);
    g.set_valign(gtk::Align::Center);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("alles-ph");
    bar.set_size_request(-1, 8);
    g.attach(&bar, 0, 0, parts, 1);
    let rest = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    g.attach(&rest, parts, 0, of - parts, 1);
    g.set_visible(false);
    g.upcast()
}

fn text_label(xalign: f32, ellipsize: gtk::pango::EllipsizeMode) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(xalign);
    l.set_ellipsize(ellipsize);
    l.set_single_line_mode(true);
    l.set_hexpand(true);
    l
}

fn new_cell(col: usize) -> AllesCell {
    let c: AllesCell = glib::Object::new();
    c.add_css_class("alles-cell");
    c.add_css_class(&format!("alles-col-{}", COLUMNS[col].0));
    c.imp().col.set(col);
    let child: gtk::Widget = match col {
        0 => {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = gtk::Image::new();
            icon.set_pixel_size(16);
            icon.add_css_class("alles-ic");
            icon.set_valign(gtk::Align::Center);
            let l = text_label(0.0, gtk::pango::EllipsizeMode::End);
            l.add_css_class("alles-nm");
            let bar = placeholder(3, 5);
            b.append(&icon);
            b.append(&l);
            b.append(&bar);
            *c.imp().icon.borrow_mut() = Some(icon);
            *c.imp().label.borrow_mut() = Some(l);
            *c.imp().bar.borrow_mut() = Some(bar);
            b.upcast()
        }
        1 => {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            // long paths lose their beginning, not the folder that matters
            let l = text_label(0.0, gtk::pango::EllipsizeMode::Start);
            l.add_css_class("alles-pathtext");
            let bar = placeholder(3, 4);
            b.append(&l);
            b.append(&bar);
            *c.imp().label.borrow_mut() = Some(l);
            *c.imp().bar.borrow_mut() = Some(bar);
            b.upcast()
        }
        2 => {
            let l = text_label(1.0, gtk::pango::EllipsizeMode::End);
            l.add_css_class("w11-tabular");
            *c.imp().label.borrow_mut() = Some(l.clone());
            l.upcast()
        }
        _ => {
            let l = text_label(0.0, gtk::pango::EllipsizeMode::End);
            l.add_css_class("w11-tabular");
            *c.imp().label.borrow_mut() = Some(l.clone());
            l.upcast()
        }
    };
    child.set_parent(&c);
    *c.imp().child.borrow_mut() = Some(child);
    if col <= 1 {
        c.set_has_tooltip(true);
        c.connect_query_tooltip(|cell, _, _, _, tip| {
            let Some(l) = cell.imp().label.borrow().clone() else { return false };
            if l.label().is_empty() || !l.layout().is_ellipsized() {
                return false;
            }
            tip.set_text(Some(&l.label()));
            true
        });
    }
    c
}

// ------------------------------------------------------------------------------------------------- icons

#[derive(Clone)]
enum IconSrc {
    Gicon(gio::Icon),
    Name(&'static str),
    Texture(gdk::Texture),
    Glyph(&'static str),
}

thread_local! {
    static ICONS: RefCell<HashMap<String, IconSrc>> = RefCell::new(HashMap::new());
    static ON_ICON: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The 16 px icon of a result (table.js iconUrl): the folder icon or the MIME type's icon from the icon theme,
/// else the backend's `/icon`, else the file / folder glyph.
fn icon_source(e: &Entry) -> IconSrc {
    let key = if e.is_dir { "\u{0}dir".to_owned() } else { e.icon_mime().to_owned() };
    if let Some(src) = ICONS.with(|c| c.borrow().get(&key).cloned()) {
        return src;
    }
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    let local = if e.is_dir {
        theme.as_ref().filter(|t| t.has_icon("folder")).map(|_| IconSrc::Name("folder"))
    } else {
        let gicon = gio::content_type_get_icon(&key);
        theme.as_ref().filter(|t| t.has_gicon(&gicon)).map(|_| IconSrc::Gicon(gicon))
    };
    let src = match local {
        Some(s) => s,
        None => {
            let fallback = if e.is_dir { "folder" } else { "file" };
            let query = if e.is_dir {
                "/icon?name=folder&size=32".to_owned()
            } else {
                format!("/icon?mime={}&size=32", kit::backend::encode(&key))
            };
            let k = key.clone();
            kit::icons::fetch(query, move |tex| {
                if let Some(tex) = tex {
                    ICONS.with(|c| c.borrow_mut().insert(k, IconSrc::Texture(tex)));
                    if let Some(f) = ON_ICON.with(|f| f.borrow().clone()) {
                        f();
                    }
                }
            });
            IconSrc::Glyph(fallback)
        }
    };
    ICONS.with(|c| {
        let mut c = c.borrow_mut();
        // a fetch answered from the kit's cache may already have stored the texture
        c.entry(key).or_insert_with(|| src.clone()).clone()
    })
}

fn set_icon(cell: &AllesCell, image: &gtk::Image, e: Option<&Entry>) {
    let Some(e) = e else {
        image.clear();
        cell.imp().icon_key.borrow_mut().clear();
        return;
    };
    let src = icon_source(e);
    let key = match &src {
        IconSrc::Gicon(g) => format!("g:{}", gio::prelude::IconExt::to_string(g).unwrap_or_default()),
        IconSrc::Name(n) => format!("n:{n}"),
        IconSrc::Texture(_) => format!("t:{}{}", e.is_dir, e.icon_mime()),
        IconSrc::Glyph(n) => format!("y:{n}"),
    };
    if *cell.imp().icon_key.borrow() == key {
        return;
    }
    *cell.imp().icon_key.borrow_mut() = key;
    match src {
        IconSrc::Gicon(g) => image.set_from_gicon(&g),
        IconSrc::Name(n) => image.set_icon_name(Some(n)),
        IconSrc::Texture(t) => image.set_paintable(Some(&t)),
        IconSrc::Glyph(n) => kit::icons::set_glyph(image, n),
    }
    image.set_pixel_size(16);
}

// ------------------------------------------------------------------------------------------------ table

struct Inner {
    view: gtk::ColumnView,
    scroller: gtk::ScrolledWindow,
    model: Results,
    selection: gtk::MultiSelection,
    columns: RefCell<Vec<gtk::ColumnViewColumn>>,
    hl: RefCell<Option<Highlight>>,
    silent: Cell<bool>,
    on_sort: RefCell<Option<Rc<dyn Fn(&str)>>>,
    on_widths: RefCell<Option<Rc<dyn Fn(HashMap<String, i32>)>>>,
}

/// The result table. Cheap to clone.
#[derive(Clone)]
pub struct ResultsTable {
    inner: Rc<Inner>,
}

impl ResultsTable {
    /// The table over `model`, with the remembered column widths.
    pub fn new(model: &Results, widths: &HashMap<String, i32>) -> ResultsTable {
        let selection = gtk::MultiSelection::new(Some(model.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.add_css_class("w11-table");
        view.add_css_class("alles-table");
        view.set_widget_name("alles-results");
        view.set_reorderable(false);
        view.set_enable_rubberband(false);
        view.set_show_row_separators(false);
        view.set_show_column_separators(false);
        view.set_single_click_activate(false);
        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("alles-scroller");
        scroller.set_child(Some(&view));
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        let inner = Rc::new(Inner {
            view: view.clone(),
            scroller,
            model: model.clone(),
            selection,
            columns: RefCell::new(Vec::new()),
            hl: RefCell::new(None),
            silent: Cell::new(false),
            on_sort: RefCell::new(None),
            on_widths: RefCell::new(None),
        });
        for (i, (id, width, min, _)) in COLUMNS.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(move |_, li| {
                let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
                li.set_child(Some(&new_cell(i)));
            });
            let weak = Rc::downgrade(&inner);
            factory.connect_bind(move |_, li| {
                let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(item), Some(cell)) = (li.item().and_downcast::<Item>(), li.child().and_downcast::<AllesCell>()) else {
                    return;
                };
                unbind(&cell);
                *cell.imp().item.borrow_mut() = Some(item.clone());
                if let Some(inner) = weak.upgrade() {
                    render(&cell, item.entry().as_deref(), inner.hl.borrow().as_ref());
                }
                let w = weak.clone();
                let cw = cell.downgrade();
                let h = item.connect_local("changed", false, move |args| {
                    if let (Some(cell), Some(inner), Ok(item)) = (cw.upgrade(), w.upgrade(), args[0].get::<Item>()) {
                        render(&cell, item.entry().as_deref(), inner.hl.borrow().as_ref());
                    }
                    None
                });
                *cell.imp().handler.borrow_mut() = Some(h);
            });
            factory.connect_unbind(|_, li| {
                if let Some(cell) = li.downcast_ref::<gtk::ListItem>().and_then(|li| li.child().and_downcast::<AllesCell>()) {
                    unbind(&cell);
                }
            });
            let column = gtk::ColumnViewColumn::new(Some(crate::text::column_label(id)), Some(factory));
            column.set_id(Some(id));
            column.set_resizable(true);
            column.set_expand(*id == "path");
            let w = widths.get(*id).copied().unwrap_or(*width).max(*min);
            column.set_fixed_width(w);
            // Only for the header's sort state; the backend orders the rows (no SortListModel).
            column.set_sorter(Some(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal)));
            let weak = Rc::downgrade(&inner);
            let min = *min;
            column.connect_fixed_width_notify(move |c| {
                if c.fixed_width() >= 0 && c.fixed_width() < min {
                    c.set_fixed_width(min);
                    return;
                }
                if let Some(inner) = weak.upgrade() {
                    ResultsTable { inner }.widths_changed();
                }
            });
            view.append_column(&column);
            inner.columns.borrow_mut().push(column);
        }
        let me = ResultsTable { inner };
        if let Some(sorter) = view.sorter() {
            let weak = Rc::downgrade(&me.inner);
            sorter.connect_changed(move |s, _| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.silent.get() {
                    return;
                }
                let Some(s) = s.downcast_ref::<gtk::ColumnViewSorter>() else { return };
                let Some(id) = s.primary_sort_column().and_then(|c| c.id()) else { return };
                let f = inner.on_sort.borrow().clone();
                if let Some(f) = f {
                    f(id.as_str());
                }
            });
        }
        let weak = Rc::downgrade(&me.inner);
        ON_ICON.with(|f| {
            *f.borrow_mut() = Some(Rc::new(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.model.refresh_all();
                }
            }))
        });
        let weak = Rc::downgrade(&me.inner);
        kit::theme::on_change(move || {
            if let Some(inner) = weak.upgrade() {
                inner.model.refresh_all();
            }
        });
        me
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.inner.scroller
    }

    pub fn view(&self) -> &gtk::ColumnView {
        &self.inner.view
    }

    pub fn selection(&self) -> &gtk::MultiSelection {
        &self.inner.selection
    }

    /// The column titles in the current language.
    pub fn relabel(&self) {
        for (c, (id, ..)) in self.inner.columns.borrow().iter().zip(COLUMNS.iter()) {
            c.set_title(Some(crate::text::column_label(id)));
        }
    }

    /// The highlight for the next rows drawn (set before a reset).
    pub fn set_highlight(&self, hl: Option<Highlight>) {
        *self.inner.hl.borrow_mut() = hl;
    }

    /// Show the sort in the headers (`by` without a column, "type": none), silently.
    pub fn set_sort(&self, by: &str, desc: bool) {
        self.inner.silent.set(true);
        let col = self.inner.columns.borrow().iter().find(|c| c.id().is_some_and(|i| i == by)).cloned();
        // one sorted column only: the sorter would keep the previous one as a secondary sort, with its arrow
        self.inner.view.sort_by_column(None, gtk::SortType::Ascending);
        if let Some(col) = col {
            self.inner.view.sort_by_column(Some(&col), if desc { gtk::SortType::Descending } else { gtk::SortType::Ascending });
        }
        self.inner.silent.set(false);
    }

    /// `f(column id)` when a header is clicked.
    pub fn connect_sort(&self, f: impl Fn(&str) + 'static) {
        *self.inner.on_sort.borrow_mut() = Some(Rc::new(f));
    }

    /// `f(widths)` after a column was resized.
    pub fn connect_widths(&self, f: impl Fn(HashMap<String, i32>) + 'static) {
        *self.inner.on_widths.borrow_mut() = Some(Rc::new(f));
    }

    fn widths_changed(&self) {
        let widths: HashMap<String, i32> = self
            .inner
            .columns
            .borrow()
            .iter()
            .filter_map(|c| Some((c.id()?.to_string(), c.fixed_width())))
            .filter(|(_, w)| *w > 0)
            .collect();
        let f = self.inner.on_widths.borrow().clone();
        if let Some(f) = f {
            f(widths);
        }
    }

    // ----------------------------------------------------------------------------------------- selection

    pub fn selected_count(&self) -> u64 {
        self.inner.selection.selection().size()
    }

    /// The selected indices in order, at most `limit`.
    pub fn selected_indices(&self, limit: usize) -> Vec<u32> {
        let set = self.inner.selection.selection();
        match gtk::BitsetIter::init_first(&set) {
            Some((iter, first)) => std::iter::once(first).chain(iter).take(limit).collect(),
            None => Vec::new(),
        }
    }

    pub fn is_selected(&self, index: u32) -> bool {
        self.inner.selection.is_selected(index)
    }

    /// Move the keyboard focus to a row; `select` also makes it the selection.
    pub fn focus_row(&self, index: u32, select: bool) {
        if index >= self.inner.model.total() {
            return;
        }
        let mut flags = gtk::ListScrollFlags::FOCUS;
        if select {
            flags |= gtk::ListScrollFlags::SELECT;
        }
        self.inner.view.scroll_to(index, None, flags, None);
    }

    /// Select several rows (the first one gets the focus).
    pub fn select_rows(&self, rows: &[u32]) {
        let total = self.inner.model.total();
        let set = gtk::Bitset::new_empty();
        for r in rows.iter().filter(|r| **r < total) {
            set.add(*r);
        }
        self.inner.selection.set_selection(&set, &gtk::Bitset::new_range(0, total));
        if let Some(first) = rows.iter().find(|r| **r < total) {
            self.inner.view.scroll_to(*first, None, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    /// Scroll a row into view.
    pub fn scroll_to_index(&self, index: u32) {
        if index < self.inner.model.total() {
            self.inner.view.scroll_to(index, None, gtk::ListScrollFlags::NONE, None);
        }
    }

    // -------------------------------------------------------------------------------------- positions

    /// The row index of a cell widget under (x, y) in the view's coordinates.
    pub fn index_at(&self, x: f64, y: f64) -> Option<u32> {
        let mut w = self.inner.view.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(cur) = w {
            if let Some(cell) = cur.downcast_ref::<AllesCell>() {
                return cell.item().map(|i| i.index());
            }
            if cur == *self.inner.view.upcast_ref::<gtk::Widget>() {
                break;
            }
            if let Some(cell) = cell_in(&cur, 2) {
                return cell.item().map(|i| i.index());
            }
            w = cur.parent();
        }
        None
    }

    /// The row a widget is (a row of this view holding the focus), and its rectangle in the view.
    pub fn row_of(&self, widget: &gtk::Widget) -> Option<(u32, gtk::graphene::Rect)> {
        let view: &gtk::Widget = self.inner.view.upcast_ref();
        let mut w = Some(widget.clone());
        while let Some(cur) = w {
            let list = cur.parent();
            if list.as_ref().and_then(|l| l.parent()).as_ref() == Some(view) {
                let cell = cell_in(&cur, 3)?;
                let index = cell.item()?.index();
                let rect = cur.compute_bounds(view)?;
                return Some((index, rect));
            }
            if cur == *view {
                return None;
            }
            w = list;
        }
        None
    }
}

/// The first AllesCell in a widget's subtree (`depth` levels).
fn cell_in(w: &gtk::Widget, depth: u32) -> Option<AllesCell> {
    if let Some(c) = w.downcast_ref::<AllesCell>() {
        return Some(c.clone());
    }
    if depth == 0 {
        return None;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(found) = cell_in(&child, depth - 1) {
            return Some(found);
        }
        c = child.next_sibling();
    }
    None
}

fn unbind(cell: &AllesCell) {
    if let Some(h) = cell.imp().handler.borrow_mut().take()
        && let Some(old) = cell.imp().item.borrow().as_ref()
    {
        old.disconnect(h);
    }
    *cell.imp().item.borrow_mut() = None;
}

fn toggle_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// Fill a cell for a result (None: a placeholder).
fn render(cell: &AllesCell, e: Option<&Entry>, hl: Option<&Highlight>) {
    let col = cell.imp().col.get();
    let label = cell.imp().label.borrow().clone();
    let bar = cell.imp().bar.borrow().clone();
    toggle_class(cell, "alles-placeholder", e.is_none());
    if let Some(b) = &bar {
        b.set_visible(e.is_none());
    }
    if let Some(l) = &label {
        l.set_visible(e.is_some() || bar.is_none());
    }
    let Some(e) = e else {
        if let Some(l) = &label {
            l.set_label("");
            l.set_attributes(None);
        }
        if let Some(img) = cell.imp().icon.borrow().as_ref() {
            set_icon(cell, img, None);
            img.set_visible(false);
        }
        return;
    };
    let Some(l) = label else { return };
    match col {
        0 => {
            if let Some(img) = cell.imp().icon.borrow().as_ref() {
                img.set_visible(true);
                set_icon(cell, img, Some(e));
            }
            highlight::set_highlighted(&l, &e.name, hl, None);
            toggle_class(cell, "alles-hidden-item", e.hidden);
            toggle_class(cell, "alles-dir", e.is_dir);
        }
        1 => {
            let dir = e.folder();
            match hl.filter(|h| h.path) {
                Some(h) => highlight::set_highlighted(&l, dir, Some(h), Some(kit::theme::token("text-1"))),
                None => highlight::set_highlighted(&l, dir, None, None),
            }
        }
        2 => {
            let t = if e.is_dir && e.size == 0.0 { String::new() } else { kit::format::bytes(e.size) };
            if l.label() != t {
                l.set_label(&t);
            }
        }
        _ => {
            let t = if e.modified > 0.0 { kit::format::file_time(e.modified) } else { String::new() };
            if l.label() != t {
                l.set_label(&t);
            }
        }
    }
}
