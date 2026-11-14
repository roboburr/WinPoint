//! The file list (vendor/ui/js/apps/explorer/fileview.js): details with sortable, resizable columns, list
//! (column flow, horizontal scrolling), small / medium / large / extra large icons, tiles and content, with
//! group headers that collapse, check boxes, cut items dimmed, hidden items faded and thumbnails from the
//! backend.
//!
//! It is virtualized: a custom Scrollable widget (the canvas, in a gtk::ScrolledWindow) computes a layout of
//! slots for every group header and item, and only the slots in view get a widget (recycled from a pool), so
//! a folder of tens of thousands of files stays fast. Selection follows Windows: click, Ctrl (toggle on
//! release), Shift (range), a click on one of several selected items selects only it on release (so a drag
//! keeps them all), rubber band on empty space (Ctrl / Shift add to it, edges scroll), keyboard (arrows,
//! Home / End, Page Up / Down, Shift / Ctrl variants, Space, Ctrl+A, type to select, Menu / Shift+F10).
//! F2-style inline rename with the validation balloon.
//!
//! For dnd.rs (builder B): `canvas()` takes the gtk::DragSource / gtk::DropTarget; `item_at(x, y)` (canvas
//! coordinates) finds the item under the pointer, `drag_paths_at(x, y)` selects it when needed and gives the
//! paths to drag (in view order), `set_dragging(paths)` dims them, `set_drop_target(Some(path) | None, whole)`
//! marks a folder (or the whole view) as the drop target. A press on an item is left to a DragSource (the
//! rubber band only starts on empty space), so return None from its `prepare` when `item_at` is None.
use crate::i18n::s;
use crate::util::{Item, display_name, flag, split_ext, st};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The view modes, in the order of Ctrl+Shift+1..8 and the View menu.
pub const VIEW_MODES: [&str; 8] = ["xl", "large", "medium", "small", "list", "details", "tiles", "content"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Xl,
    Large,
    Medium,
    Small,
    List,
    Details,
    Tiles,
    Content,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "xl" => Mode::Xl,
            "large" => Mode::Large,
            "medium" => Mode::Medium,
            "small" => Mode::Small,
            "list" => Mode::List,
            "tiles" => Mode::Tiles,
            "content" => Mode::Content,
            _ => Mode::Details,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Xl => "xl",
            Mode::Large => "large",
            Mode::Medium => "medium",
            Mode::Small => "small",
            Mode::List => "list",
            Mode::Details => "details",
            Mode::Tiles => "tiles",
            Mode::Content => "content",
        }
    }

    /// fileview.js ICON_SIZE.
    pub fn icon_size(self) -> i32 {
        match self {
            Mode::Xl => 256,
            Mode::Large => 96,
            Mode::Medium | Mode::Tiles => 48,
            Mode::Content => 32,
            _ => 16,
        }
    }

    /// Up / Down follow the list order (no geometry).
    fn order_nav(self) -> bool {
        matches!(self, Mode::Details | Mode::List | Mode::Content)
    }

    /// The icon views with a name under the icon (two lines, a multi-line rename box).
    fn stacked(self) -> bool {
        matches!(self, Mode::Xl | Mode::Large | Mode::Medium)
    }
}

/// A details column.
#[derive(Clone)]
pub struct Column {
    pub id: String,
    pub label: String,
    pub width: f64,
    pub min: f64,
    pub align_end: bool,
    /// The cell text (unused for the name column).
    pub render: Rc<dyn Fn(&serde_json::Value) -> String>,
}

/// A group of the "Groeperen op" view.
#[derive(Clone)]
pub struct Group {
    pub key: String,
    pub label: String,
    pub items: Vec<Item>,
}

type ItemFn = Rc<dyn Fn(&Item)>;
/// `done(Ok)` when renamed, `done(Err(message))` to keep the box open with that message.
pub type RenameDone = Box<dyn FnOnce(Result<(), String>)>;

/// What the view tells its owner.
#[derive(Clone)]
pub struct Handlers {
    pub on_open: ItemFn,
    pub on_middle: ItemFn,
    /// The selection changed (selection, focused path).
    pub on_select: Rc<dyn Fn(&HashSet<String>, Option<String>)>,
    /// A context menu at (x, y) in canvas coordinates, for an item (the selection) or the background.
    pub on_context: Rc<dyn Fn(f64, f64, Option<Item>)>,
    pub on_rename: Rc<dyn Fn(&Item, &str, RenameDone)>,
    pub on_rename_end: Rc<dyn Fn()>,
    pub on_sort: Rc<dyn Fn(&str)>,
    pub on_column_resize: Rc<dyn Fn(&str, f64)>,
    pub on_header_context: Rc<dyn Fn(f64, f64)>,
    /// Ctrl + wheel: +1 bigger icons, -1 smaller.
    pub on_zoom: Rc<dyn Fn(i32)>,
}

impl Default for Handlers {
    fn default() -> Handlers {
        Handlers {
            on_open: Rc::new(|_| {}),
            on_middle: Rc::new(|_| {}),
            on_select: Rc::new(|_, _| {}),
            on_context: Rc::new(|_, _, _| {}),
            on_rename: Rc::new(|_, _, done| done(Err(String::new()))),
            on_rename_end: Rc::new(|| {}),
            on_sort: Rc::new(|_| {}),
            on_column_resize: Rc::new(|_, _| {}),
            on_header_context: Rc::new(|_, _| {}),
            on_zoom: Rc::new(|_| {}),
        }
    }
}

/// How a view starts (fileview.js FileView(o)).
pub struct ViewOpts {
    pub mode: Mode,
    pub items: Vec<Item>,
    pub columns: Vec<Column>,
    pub groups: Option<Vec<Group>>,
    pub selection: HashSet<String>,
    pub focus: Option<String>,
    pub cut: HashSet<String>,
    /// (column id, descending)
    pub sort: (String, bool),
    pub compact: bool,
    pub checkboxes: bool,
    pub show_ext: bool,
    pub empty: Option<gtk::Widget>,
    /// Collapsed group keys (kept by the owner per location).
    pub collapsed: Rc<RefCell<HashSet<String>>>,
    pub label: String,
    pub src_dir: Option<String>,
    pub draggable: bool,
    pub type_of: Rc<dyn Fn(&serde_json::Value) -> String>,
    pub size_of: Rc<dyn Fn(&serde_json::Value) -> String>,
    pub content_props: Rc<dyn Fn(&serde_json::Value) -> (String, String)>,
    pub handlers: Handlers,
}

// ------------------------------------------------------------------------------------------- the canvas

mod imp {
    use super::*;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) hadj: RefCell<Option<gtk::Adjustment>>,
        pub(super) vadj: RefCell<Option<gtk::Adjustment>>,
        pub(super) hpol: Cell<Option<gtk::ScrollablePolicy>>,
        pub(super) vpol: Cell<Option<gtk::ScrollablePolicy>>,
        pub(super) hsig: RefCell<Option<glib::SignalHandlerId>>,
        pub(super) vsig: RefCell<Option<glib::SignalHandlerId>>,
        pub(super) view: RefCell<Weak<super::Inner>>,
        /// Children drawn above the others (the column header, the rubber band).
        pub(super) top: RefCell<Vec<gtk::Widget>>,
        pub(super) configuring: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "W11ExplorerCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);
    }

    impl ObjectImpl for Canvas {
        fn properties() -> &'static [glib::ParamSpec] {
            static P: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            P.get_or_init(|| {
                vec![
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hscroll-policy"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vscroll-policy"),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "hadjustment" => self.set_adj(true, value.get::<Option<gtk::Adjustment>>().ok().flatten()),
                "vadjustment" => self.set_adj(false, value.get::<Option<gtk::Adjustment>>().ok().flatten()),
                "hscroll-policy" => self.hpol.set(value.get::<gtk::ScrollablePolicy>().ok()),
                "vscroll-policy" => self.vpol.set(value.get::<gtk::ScrollablePolicy>().ok()),
                _ => {}
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "hadjustment" => self.hadj.borrow().to_value(),
                "vadjustment" => self.vadj.borrow().to_value(),
                "hscroll-policy" => self.hpol.get().unwrap_or(gtk::ScrollablePolicy::Minimum).to_value(),
                _ => self.vpol.get().unwrap_or(gtk::ScrollablePolicy::Minimum).to_value(),
            }
        }

        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl Canvas {
        fn set_adj(&self, horizontal: bool, adj: Option<gtk::Adjustment>) {
            let (slot, sig) = if horizontal { (&self.hadj, &self.hsig) } else { (&self.vadj, &self.vsig) };
            if let (Some(old), Some(id)) = (slot.borrow_mut().take(), sig.borrow_mut().take()) {
                old.disconnect(id);
            }
            if let Some(a) = &adj {
                let weak = self.obj().downgrade();
                let id = a.connect_value_changed(move |_| {
                    if let Some(c) = weak.upgrade()
                        && !c.imp().configuring.get()
                    {
                        c.queue_allocate();
                        if let Some(v) = c.imp().view.borrow().upgrade() {
                            v.scrolled();
                        }
                    }
                });
                *sig.borrow_mut() = Some(id);
            }
            *slot.borrow_mut() = adj;
            self.obj().queue_allocate();
        }
    }

    impl WidgetImpl for Canvas {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let view = self.view.borrow().upgrade();
            if let Some(v) = view {
                v.allocate(width, height);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let top = self.top.borrow();
            let mut c = obj.first_child();
            while let Some(child) = c {
                if !top.contains(&child) {
                    obj.snapshot_child(&child, snapshot);
                }
                c = child.next_sibling();
            }
            for t in top.iter() {
                obj.snapshot_child(t, snapshot);
            }
        }
    }

    impl ScrollableImpl for Canvas {}
}

glib::wrapper! {
    /// The scrollable surface the file view draws its slots on.
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Canvas {
    fn new() -> Canvas {
        glib::Object::new()
    }

    fn adjustments(&self) -> (Option<gtk::Adjustment>, Option<gtk::Adjustment>) {
        (self.imp().hadj.borrow().clone(), self.imp().vadj.borrow().clone())
    }

    fn scroll_pos(&self) -> (f64, f64) {
        let (h, v) = self.adjustments();
        (h.map(|a| a.value()).unwrap_or(0.0), v.map(|a| a.value()).unwrap_or(0.0))
    }
}

// ------------------------------------------------------------------------------------------------ layout

const HEADER_H: f64 = 32.0;
const CHECK_COL: f64 = 28.0;
const OVERSCAN: f64 = 240.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum SlotKind {
    Group(usize),
    Item(usize),
    Empty,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    kind: SlotKind,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Slot {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A run of the visual order: one group (None: ungrouped) and its items (none when collapsed).
struct Section {
    group: Option<usize>,
    start: usize,
    len: usize,
}

// ------------------------------------------------------------------------------------------ item widgets

struct ItemCell {
    root: gtk::Overlay,
    icon: gtk::Box,
    name_box: gtk::Box,
    name: gtk::Label,
    subs: Vec<gtk::Label>,
    /// Details: the box of every column (name first) and the text label of the other columns.
    col_boxes: Vec<gtk::Box>,
    col_labels: Vec<gtk::Label>,
    check: Option<gtk::Box>,
    bound: RefCell<Option<String>>,
}

struct GroupRow {
    root: gtk::Box,
    chev: gtk::Image,
    label: gtk::Label,
    count: gtk::Label,
}

#[derive(Clone)]
enum Live {
    Item(Rc<ItemCell>),
    Group(Rc<GroupRow>),
    Empty,
}

fn check_box(px: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("xp-check");
    b.set_size_request(16, 16);
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    b.set_hexpand(false);
    let g = kit::icons::glyph_stroke("check", px, 1.6);
    g.add_css_class("xp-check-glyph");
    g.set_hexpand(true);
    b.append(&g);
    b
}

fn label(class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_single_line_mode(true);
    for c in class.split_whitespace() {
        l.add_css_class(c);
    }
    l
}

// ----------------------------------------------------------------------------------------------- the view

struct Band {
    x0: f64,
    y0: f64,
    /// Canvas coordinates of the start and the last pointer position.
    start: (f64, f64),
    last: (f64, f64),
    base: HashSet<String>,
    moved: bool,
    timer: Option<glib::SourceId>,
}

struct Rename {
    path: String,
    item: Item,
    entry: gtk::Widget,
    tip: gtk::Popover,
    tip_label: gtk::Label,
    hide_ext: bool,
    ext: String,
    busy: bool,
}

pub(crate) struct Inner {
    mode: Mode,
    items: Vec<Item>,
    by_path: HashMap<String, Item>,
    columns: Vec<Column>,
    groups: Vec<Group>,
    sort: (String, bool),
    compact: bool,
    checkboxes: bool,
    show_ext: bool,
    draggable: bool,
    collapsed: Rc<RefCell<HashSet<String>>>,
    empty: Option<gtk::Widget>,
    src_dir: Option<String>,
    type_of: Rc<dyn Fn(&serde_json::Value) -> String>,
    size_of: Rc<dyn Fn(&serde_json::Value) -> String>,
    content_props: Rc<dyn Fn(&serde_json::Value) -> (String, String)>,
    h: Handlers,

    root: gtk::Box,
    canvas: Canvas,
    header: RefCell<Option<gtk::Box>>,
    header_cells: RefCell<Vec<gtk::Box>>,
    header_check: RefCell<Option<gtk::Box>>,
    band_widget: gtk::Box,

    col_widths: RefCell<Vec<f64>>,
    order: RefCell<Vec<Item>>,
    index: RefCell<HashMap<String, usize>>,
    sections: RefCell<Vec<Section>>,
    slots: RefCell<Vec<Slot>>,
    item_slot: RefCell<Vec<usize>>,
    content_size: Cell<(f64, f64)>,
    view_size: Cell<(i32, i32)>,
    layout_key: Cell<(i32, i32)>,
    dirty: Cell<bool>,

    live: RefCell<HashMap<usize, Live>>,
    pool: RefCell<Vec<Rc<ItemCell>>>,
    all_cells: RefCell<Vec<Rc<ItemCell>>>,
    gpool: RefCell<Vec<Rc<GroupRow>>>,

    selection: RefCell<HashSet<String>>,
    focus: RefCell<Option<String>>,
    anchor: RefCell<Option<String>>,
    cut: RefCell<HashSet<String>>,
    drop_mark: RefCell<Option<String>>,
    dragging: RefCell<HashSet<String>>,
    pending: RefCell<Option<(bool, String)>>,
    band: RefCell<Option<Band>>,
    rename: RefCell<Option<Rename>>,
    type_buf: RefCell<String>,
    type_timer: RefCell<Option<glib::SourceId>>,
    pending_restore: Cell<Option<(f64, f64)>>,
    pending_reveal: RefCell<Option<(String, bool)>>,
    after_layout: RefCell<Vec<Box<dyn FnOnce()>>>,
    destroyed: Cell<bool>,
}

/// A file view; a cheap handle (clone).
#[derive(Clone)]
pub struct FileView {
    inner: Rc<Inner>,
}

impl FileView {
    pub fn new(o: ViewOpts) -> FileView {
        let mode = o.mode;
        let by_path: HashMap<String, Item> = o.items.iter().map(|it| (st(it, "path").to_owned(), it.clone())).collect();
        let selection: HashSet<String> = o.selection.into_iter().filter(|p| by_path.contains_key(p)).collect();
        let focus = o.focus.filter(|p| by_path.contains_key(p));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("xp-view");
        root.add_css_class(&format!("xp-mode-{}", mode.as_str()));
        if o.compact {
            root.add_css_class("is-compact");
        }
        if o.checkboxes {
            root.add_css_class("has-checks");
        }
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_widget_name("explorer-view");
        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("xp-scroll");
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        if mode == Mode::List {
            scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        }
        let canvas = Canvas::new();
        canvas.add_css_class("xp-canvas");
        canvas.set_focusable(true);
        canvas.set_overflow(gtk::Overflow::Hidden);
        canvas.set_widget_name("explorer-items");
        canvas.update_property(&[gtk::accessible::Property::Label(&o.label), gtk::accessible::Property::MultiSelectable(true)]);
        scroller.set_child(Some(&canvas));
        root.append(&scroller);

        let band_widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        band_widget.add_css_class("xp-band");
        band_widget.set_can_target(false);
        band_widget.set_parent(&canvas);
        band_widget.set_child_visible(false);

        let col_widths: Vec<f64> = o.columns.iter().map(|c| c.width.max(c.min.max(60.0))).collect();
        let groups = o.groups.unwrap_or_default();
        if let Some(e) = &o.empty {
            e.set_parent(&canvas);
            e.set_child_visible(false);
        }
        let inner = Rc::new(Inner {
            mode,
            items: o.items,
            by_path,
            columns: o.columns,
            groups,
            sort: o.sort,
            compact: o.compact,
            checkboxes: o.checkboxes,
            show_ext: o.show_ext,
            draggable: o.draggable,
            collapsed: o.collapsed,
            empty: o.empty,
            src_dir: o.src_dir,
            type_of: o.type_of,
            size_of: o.size_of,
            content_props: o.content_props,
            h: o.handlers,
            root,
            canvas: canvas.clone(),
            header: RefCell::new(None),
            header_cells: RefCell::new(Vec::new()),
            header_check: RefCell::new(None),
            band_widget: band_widget.clone(),
            col_widths: RefCell::new(col_widths),
            order: RefCell::new(Vec::new()),
            index: RefCell::new(HashMap::new()),
            sections: RefCell::new(Vec::new()),
            slots: RefCell::new(Vec::new()),
            item_slot: RefCell::new(Vec::new()),
            content_size: Cell::new((0.0, 0.0)),
            view_size: Cell::new((0, 0)),
            layout_key: Cell::new((-1, -1)),
            dirty: Cell::new(true),
            live: RefCell::new(HashMap::new()),
            pool: RefCell::new(Vec::new()),
            all_cells: RefCell::new(Vec::new()),
            gpool: RefCell::new(Vec::new()),
            selection: RefCell::new(selection),
            anchor: RefCell::new(focus.clone()),
            focus: RefCell::new(focus),
            cut: RefCell::new(o.cut),
            drop_mark: RefCell::new(None),
            dragging: RefCell::new(HashSet::new()),
            pending: RefCell::new(None),
            band: RefCell::new(None),
            rename: RefCell::new(None),
            type_buf: RefCell::new(String::new()),
            type_timer: RefCell::new(None),
            pending_restore: Cell::new(None),
            pending_reveal: RefCell::new(None),
            after_layout: RefCell::new(Vec::new()),
            destroyed: Cell::new(false),
        });
        // the header needs `inner` for its handlers
        if mode == Mode::Details {
            let (header, cells, check) = build_header(&inner);
            header.set_parent(&canvas);
            canvas.imp().top.borrow_mut().push(header.clone().upcast());
            *inner.header.borrow_mut() = Some(header);
            *inner.header_cells.borrow_mut() = cells;
            *inner.header_check.borrow_mut() = check;
        }
        canvas.imp().top.borrow_mut().push(band_widget.upcast());
        *canvas.imp().view.borrow_mut() = Rc::downgrade(&inner);
        let me = FileView { inner };
        let header = me.inner.header.borrow().clone();
        if let Some(h) = header {
            me.wire_header(&h);
        }
        me.inner.rebuild_order();
        me.install_events();
        me.inner.paint();
        me
    }

    /// The whole view (header, items, scroll bars).
    pub fn widget(&self) -> gtk::Widget {
        self.inner.root.clone().upcast()
    }

    /// The scrollable surface (event controllers for drag and drop go here).
    pub fn canvas(&self) -> gtk::Widget {
        self.inner.canvas.clone().upcast()
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn mode(&self) -> Mode {
        self.inner.mode
    }

    /// Whether two handles are the same view.
    pub fn same(&self, other: &FileView) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// The folder the items come from (None in search results and the Prullenbak).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn src_dir(&self) -> Option<String> {
        self.inner.src_dir.clone()
    }

    /// Whether items may be dragged (not in the Prullenbak).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn draggable(&self) -> bool {
        self.inner.draggable
    }

    /// The items in visual order (collapsed groups left out).
    pub fn order(&self) -> Vec<Item> {
        self.inner.order.borrow().clone()
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn selection(&self) -> HashSet<String> {
        self.inner.selection.borrow().clone()
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn focus_path(&self) -> Option<String> {
        self.inner.focus.borrow().clone()
    }

    /// Whether the inline rename box is open.
    pub fn renaming(&self) -> bool {
        self.inner.rename.borrow().is_some()
    }

    /// Focus the list (the keyboard continues from the focused item).
    pub fn focus(&self) {
        self.inner.canvas.grab_focus();
    }

    /// Select these paths (unknown ones are skipped); `focus` becomes the focused item, `scroll` brings it in
    /// view. Runs the selection handler.
    pub fn select(&self, paths: &[String], focus: Option<&str>, scroll: bool) {
        self.inner.select(paths.iter().cloned(), focus.map(str::to_owned), scroll, false);
    }

    pub fn select_all(&self) {
        let all: Vec<String> = self.inner.order.borrow().iter().map(|x| st(x, "path").to_owned()).collect();
        let f = self.inner.focus.borrow().clone();
        self.inner.select(all.into_iter(), f, false, false);
    }

    pub fn invert(&self) {
        let sel = self.inner.selection.borrow().clone();
        let all: Vec<String> =
            self.inner.order.borrow().iter().map(|x| st(x, "path").to_owned()).filter(|p| !sel.contains(p)).collect();
        let f = self.inner.focus.borrow().clone();
        self.inner.select(all.into_iter(), f, false, false);
    }

    /// Open the inline rename box on an item (false when it is not in the view).
    pub fn start_rename(&self, path: &str) -> bool {
        start_rename(&self.inner, path)
    }

    /// Scroll an item into view (once laid out); `focus`: give the keyboard focus to the list too.
    pub fn reveal(&self, path: &str, focus: bool) {
        if !self.inner.by_path.contains_key(path) {
            return;
        }
        if self.inner.layout_key.get().0 < 0 {
            *self.inner.pending_reveal.borrow_mut() = Some((path.to_owned(), focus));
            return;
        }
        self.inner.scroll_to(path);
        if focus {
            self.inner.canvas.grab_focus();
        }
    }

    /// Dim the items on the clipboard after Knippen.
    pub fn set_cut(&self, set: HashSet<String>) {
        *self.inner.cut.borrow_mut() = set;
        self.inner.paint();
    }

    /// The scroll position (top, left).
    pub fn scroll(&self) -> (f64, f64) {
        let (h, v) = self.inner.canvas.scroll_pos();
        (v, h)
    }

    /// Scroll to (top, left) once laid out.
    pub fn restore(&self, pos: (f64, f64)) {
        if self.inner.layout_key.get().0 < 0 {
            self.inner.pending_restore.set(Some(pos));
        } else {
            self.inner.apply_scroll(pos);
        }
    }

    /// The item at (x, y) in canvas coordinates (None on empty space, a group header or the column header).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn item_at(&self, x: f64, y: f64) -> Option<Item> {
        match self.inner.hit(x, y) {
            Some(SlotKind::Item(i)) => self.inner.order.borrow().get(i).cloned(),
            _ => None,
        }
    }

    /// For a drag that starts at (x, y): select the item there unless it is selected, and give the selected
    /// paths in view order (empty on empty space or while renaming).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn drag_paths_at(&self, x: f64, y: f64) -> Vec<String> {
        if self.renaming() || !self.inner.draggable {
            return Vec::new();
        }
        let Some(item) = self.item_at(x, y) else { return Vec::new() };
        let path = st(&item, "path").to_owned();
        *self.inner.pending.borrow_mut() = None;
        if !self.inner.selection.borrow().contains(&path) {
            self.inner.select(std::iter::once(path.clone()), Some(path), false, false);
        }
        let sel = self.inner.selection.borrow().clone();
        self.inner.order.borrow().iter().map(|x| st(x, "path").to_owned()).filter(|p| sel.contains(p)).collect()
    }

    /// Dim the items being dragged (empty: none).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn set_dragging(&self, paths: &[String]) {
        *self.inner.dragging.borrow_mut() = paths.iter().cloned().collect();
        self.inner.paint();
    }

    /// Mark a folder item as the drop target, or with `whole` the view itself; None, false clears it.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn set_drop_target(&self, path: Option<&str>, whole: bool) {
        *self.inner.drop_mark.borrow_mut() = path.map(str::to_owned);
        if whole && path.is_none() {
            self.inner.root.add_css_class("is-drop");
        } else {
            self.inner.root.remove_css_class("is-drop");
        }
        self.inner.paint();
    }

    /// Run `f` once the view has its first layout (scroll, reveal, rename asked right after creating it).
    pub fn after_layout(&self, f: impl FnOnce() + 'static) {
        if self.inner.layout_key.get().0 >= 0 {
            glib::idle_add_local_once(f);
        } else {
            self.inner.after_layout.borrow_mut().push(Box::new(f));
        }
    }

    /// Stop timers (the view is being replaced).
    pub fn destroy(&self) {
        let i = &self.inner;
        i.destroyed.set(true);
        if let Some(id) = i.type_timer.borrow_mut().take() {
            id.remove();
        }
        if let Some(b) = i.band.borrow_mut().as_mut()
            && let Some(id) = b.timer.take()
        {
            id.remove();
        }
        let r = i.rename.borrow_mut().take();
        if let Some(r) = r {
            r.tip.unparent();
        }
        i.after_layout.borrow_mut().clear();
    }

    fn weak(&self) -> Weak<Inner> {
        Rc::downgrade(&self.inner)
    }

    // ------------------------------------------------------------------------------------- the header

    fn wire_header(&self, header: &gtk::Box) {
        let ctx_click = gtk::GestureClick::new();
        ctx_click.set_button(gdk::BUTTON_SECONDARY);
        let weak = self.weak();
        ctx_click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(i) = weak.upgrade()
                && let Some(w) = g.widget()
                && let Some(p) = w.compute_point(&i.canvas, &gtk::graphene::Point::new(x as f32, y as f32))
            {
                (i.h.on_header_context)(p.x() as f64, p.y() as f64);
            }
        });
        header.add_controller(ctx_click);
        // a press on the header never starts the rubber band
        let block = gtk::GestureClick::new();
        block.connect_pressed(|g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        header.add_controller(block);
    }

    // ---------------------------------------------------------------------------------------- events

    fn install_events(&self) {
        let canvas = &self.inner.canvas;
        // clicks: selection, open, middle, context menu
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = self.weak();
        click.connect_pressed(move |g, n, x, y| {
            if let Some(i) = weak.upgrade() {
                on_press(&i, g, n, x, y);
            }
        });
        let weak = self.weak();
        click.connect_released(move |g, _, x, y| {
            if let Some(i) = weak.upgrade() {
                on_release(&i, g, x, y);
            }
        });
        let weak = self.weak();
        click.connect_stopped(move |_| {
            if let Some(i) = weak.upgrade() {
                *i.pending.borrow_mut() = None;
            }
        });
        canvas.add_controller(click);

        // rubber band on empty space
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let weak = self.weak();
        drag.connect_drag_begin(move |g, x, y| {
            if let Some(i) = weak.upgrade() {
                band_begin(&i, g, x, y);
            }
        });
        let weak = self.weak();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(i) = weak.upgrade() {
                band_update(&i, Some((dx, dy)));
            }
        });
        let weak = self.weak();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(i) = weak.upgrade() {
                band_end(&i);
            }
        });
        canvas.add_controller(drag);

        // keyboard
        let keys = gtk::EventControllerKey::new();
        let weak = self.weak();
        keys.connect_key_pressed(move |_, key, _, state| match weak.upgrade() {
            Some(i) => on_key(&i, key, state),
            None => glib::Propagation::Proceed,
        });
        canvas.add_controller(keys);

        // Ctrl + wheel zooms; in the list view the wheel scrolls sideways
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = self.weak();
        scroll.connect_scroll(move |c, _, dy| {
            let Some(i) = weak.upgrade() else { return glib::Propagation::Proceed };
            if c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                if dy != 0.0 {
                    (i.h.on_zoom)(if dy < 0.0 { 1 } else { -1 });
                }
                return glib::Propagation::Stop;
            }
            if i.mode == Mode::List
                && let (Some(h), _) = i.canvas.adjustments()
            {
                let step = 48.0 * dy;
                h.set_value((h.value() + step).clamp(h.lower(), (h.upper() - h.page_size()).max(0.0)));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        canvas.add_controller(scroll);
    }
}

// --------------------------------------------------------------------------------------- header building

fn build_header(i: &Rc<Inner>) -> (gtk::Box, Vec<gtk::Box>, Option<gtk::Box>) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header.add_css_class("xp-head");
    header.set_widget_name("explorer-columns");
    let mut check = None;
    if i.checkboxes {
        let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        wrap.set_size_request(CHECK_COL as i32, -1);
        wrap.set_hexpand(false);
        let c = check_box(12);
        c.add_css_class("xp-head-check");
        c.set_hexpand(true);
        wrap.append(&c);
        let weak = Rc::downgrade(i);
        let g = gtk::GestureClick::new();
        g.connect_released(move |_, _, _, _| {
            if let Some(i) = weak.upgrade() {
                let all = i.selection.borrow().len() == i.items.len();
                let f = i.focus.borrow().clone();
                if all {
                    i.select(std::iter::empty(), f, false, false);
                } else {
                    let paths: Vec<String> = i.items.iter().map(|x| st(x, "path").to_owned()).collect();
                    i.select(paths.into_iter(), f, false, false);
                }
            }
        });
        c.add_controller(g);
        header.append(&wrap);
        check = Some(c);
    }
    let mut cells = Vec::new();
    let widths = i.col_widths.borrow().clone();
    for (ci, col) in i.columns.iter().enumerate() {
        // A header cell is exactly its column's width, as the item cells below it (make_cell): never wider. The
        // header box is allocated the view's width, so a cell that expanded (the label's hexpand propagates) or
        // asked for its label's natural width took a share of the spare room and drifted off its column.
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("xp-head-cell");
        cell.set_size_request(widths[ci].round() as i32, -1);
        cell.set_hexpand(false);
        let sorted = i.sort.0 == col.id;
        if sorted {
            cell.add_css_class("is-sorted");
        }
        if col.align_end {
            cell.add_css_class("is-end");
        }
        let ov = gtk::Overlay::new();
        ov.set_hexpand(true);
        let l = label("xp-head-label");
        l.set_label(&col.label);
        if col.align_end {
            l.set_xalign(1.0);
        }
        l.set_hexpand(true);
        // natural width one character: the cell's width request decides, a long title ellipsizes
        l.set_max_width_chars(1);
        ov.set_child(Some(&l));
        if sorted {
            let g = kit::icons::glyph_stroke(if i.sort.1 { "chevron-down" } else { "chevron-up" }, 10, 1.3);
            g.add_css_class("xp-sortglyph");
            g.set_halign(gtk::Align::Center);
            g.set_valign(gtk::Align::Start);
            ov.add_overlay(&g);
        }
        cell.append(&ov);
        let grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grip.add_css_class("xp-head-grip");
        grip.set_size_request(7, -1);
        grip.set_cursor_from_name(Some("col-resize"));
        cell.append(&grip);
        // sort on click (not on the grip)
        let id = col.id.clone();
        let weak = Rc::downgrade(i);
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let gw = grip.downgrade();
        click.connect_released(move |g, _, x, y| {
            let Some(i) = weak.upgrade() else { return };
            if let (Some(grip), Some(cell)) = (gw.upgrade(), g.widget())
                && let Some(p) = cell.compute_point(&grip, &gtk::graphene::Point::new(x as f32, y as f32))
                && grip.contains(p.x() as f64, p.y() as f64)
            {
                return;
            }
            (i.h.on_sort)(&id);
        });
        cell.add_controller(click);
        // resize by dragging the grip, auto-fit on double click. The pointer is followed in canvas coordinates:
        // the drag's own offsets are in the grip's, and the grip moves with the column's edge, so the edge went
        // at half the pointer's speed.
        let drag = gtk::GestureDrag::new();
        let w0 = Rc::new(Cell::new(0.0));
        let x0 = Rc::new(Cell::new(0.0));
        let weak = Rc::downgrade(i);
        let (w0b, x0b) = (w0.clone(), x0.clone());
        drag.connect_drag_begin(move |g, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(i) = weak.upgrade() {
                w0b.set(i.col_widths.borrow()[ci]);
                x0b.set(canvas_x(&i, g, x, y));
                i.root.add_css_class("is-resizing");
            }
        });
        let weak = Rc::downgrade(i);
        let min = col.min.max(60.0);
        let (w0c, x0c) = (w0.clone(), x0.clone());
        drag.connect_drag_update(move |g, _, _| {
            if let Some(i) = weak.upgrade()
                && let Some((x, y)) = g.point(g.current_sequence().as_ref())
            {
                let dx = canvas_x(&i, g, x, y) - x0c.get();
                i.set_col_width(ci, (w0c.get() + dx).clamp(min, 1200.0));
            }
        });
        let weak = Rc::downgrade(i);
        let cid = col.id.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(i) = weak.upgrade() {
                i.root.remove_css_class("is-resizing");
                let w = i.col_widths.borrow()[ci];
                (i.h.on_column_resize)(&cid, w);
            }
        });
        // One group with the drag: a gesture that claims the press denies the other gestures of the widget that
        // are not grouped with it, so the drag and the double click would each have stopped the other.
        let dbl = gtk::GestureClick::new();
        grip.add_controller(drag.clone());
        let weak = Rc::downgrade(i);
        let cid = col.id.clone();
        let w0d = w0.clone();
        dbl.connect_pressed(move |g, n, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if n != 2 {
                return;
            }
            if let Some(i) = weak.upgrade() {
                let w = i.fit_width(ci).clamp(min, 900.0);
                // the drag of this second press starts from the fitted width (a jitter keeps the fit)
                w0d.set(w);
                i.set_col_width(ci, w);
                (i.h.on_column_resize)(&cid, w);
            }
        });
        grip.add_controller(dbl.clone());
        dbl.group_with(&drag);
        header.append(&cell);
        cells.push(cell);
    }
    let fill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    fill.set_hexpand(true);
    fill.add_css_class("xp-head-fill");
    header.append(&fill);
    (header, cells, check)
}

/// A point of a gesture on a header grip in canvas coordinates (the grip itself moves while it resizes).
fn canvas_x(i: &Inner, g: &gtk::GestureDrag, x: f64, y: f64) -> f64 {
    g.widget()
        .and_then(|w| w.compute_point(&i.canvas, &gtk::graphene::Point::new(x as f32, y as f32)))
        .map_or(x, |p| p.x() as f64)
}

// ------------------------------------------------------------------------------------------- Inner impl

impl Inner {
    fn rebuild_order(&self) {
        let collapsed = self.collapsed.borrow();
        let mut order = Vec::new();
        let mut sections = Vec::new();
        let labeled = self.groups.iter().any(|g| !g.label.is_empty());
        if !self.groups.is_empty() && labeled {
            for (gi, g) in self.groups.iter().enumerate() {
                let start = order.len();
                if !g.label.is_empty() && collapsed.contains(&g.key) {
                    sections.push(Section { group: Some(gi), start, len: 0 });
                    continue;
                }
                order.extend(g.items.iter().cloned());
                sections.push(Section { group: if g.label.is_empty() { None } else { Some(gi) }, start, len: g.items.len() });
            }
        } else {
            order.extend(self.items.iter().cloned());
            sections.push(Section { group: None, start: 0, len: order.len() });
        }
        *self.index.borrow_mut() = order.iter().enumerate().map(|(n, it)| (st(it, "path").to_owned(), n)).collect();
        *self.order.borrow_mut() = order;
        *self.sections.borrow_mut() = sections;
        self.dirty.set(true);
        // focus the first item when nothing is (a fresh folder: no ring until keyboard use)
        let first = self.order.borrow().first().map(|f| st(f, "path").to_owned());
        if self.focus.borrow().is_none() {
            *self.focus.borrow_mut() = first;
        }
    }

    fn cols_w(&self) -> f64 {
        self.col_widths.borrow().iter().sum::<f64>() + if self.checkboxes { CHECK_COL } else { 0.0 } + 8.0
    }

    fn row_h(&self) -> f64 {
        match self.mode {
            Mode::Details => {
                if self.compact {
                    24.0
                } else {
                    32.0
                }
            }
            Mode::List | Mode::Small => 24.0,
            Mode::Content => 64.0,
            Mode::Tiles => 72.0,
            Mode::Medium => 96.0,
            Mode::Large => 144.0,
            Mode::Xl => 310.0,
        }
    }

    fn top_inset(&self) -> f64 {
        if self.mode == Mode::Details { HEADER_H } else { 0.0 }
    }

    fn layout(&self, w: i32, h: i32) {
        let w = w.max(1) as f64;
        let h = h.max(1) as f64;
        let order_len = self.order.borrow().len();
        let mut slots: Vec<Slot> = Vec::with_capacity(order_len + 8);
        let mut item_slot = vec![0usize; order_len];
        let sections = self.sections.borrow();
        let row_h = self.row_h();
        let mut push_item = |slots: &mut Vec<Slot>, idx: usize, x: f64, y: f64, sw: f64, sh: f64| {
            item_slot[idx] = slots.len();
            slots.push(Slot { kind: SlotKind::Item(idx), x, y, w: sw, h: sh });
        };
        let (cw, ch);
        match self.mode {
            Mode::Details => {
                let cols_w = self.cols_w();
                let (x0, iw) = (8.0, cols_w - 8.0);
                let mut y = HEADER_H;
                for sec in sections.iter() {
                    if let Some(g) = sec.group {
                        y += 4.0;
                        slots.push(Slot { kind: SlotKind::Group(g), x: x0, y, w: iw, h: 36.0 });
                        y += 36.0;
                    }
                    for idx in sec.start..sec.start + sec.len {
                        push_item(&mut slots, idx, x0, y, iw, row_h);
                        y += row_h;
                    }
                }
                cw = (cols_w + 8.0).max(w);
                ch = y + 16.0;
            }
            Mode::Content => {
                let (x0, iw) = (12.0, (w - 24.0).max(200.0));
                let mut y = 4.0;
                for sec in sections.iter() {
                    if let Some(g) = sec.group {
                        y += 4.0;
                        slots.push(Slot { kind: SlotKind::Group(g), x: x0, y, w: iw, h: 36.0 });
                        y += 36.0;
                    }
                    for idx in sec.start..sec.start + sec.len {
                        push_item(&mut slots, idx, x0, y, iw, row_h);
                        y += row_h;
                    }
                }
                cw = (iw + 24.0).max(w);
                ch = y + 20.0;
            }
            Mode::List => {
                let (col_w, gap, top) = (240.0, 12.0, 8.0);
                let bottom = (h - 8.0).max(top + 40.0);
                let (mut x, mut y) = (12.0, top);
                for sec in sections.iter() {
                    if let Some(g) = sec.group {
                        if y + 40.0 > bottom && y > top {
                            x += col_w + gap;
                            y = top;
                        }
                        slots.push(Slot { kind: SlotKind::Group(g), x, y: y + 4.0, w: col_w, h: 36.0 });
                        y += 40.0;
                    }
                    for idx in sec.start..sec.start + sec.len {
                        if y + row_h > bottom && y > top {
                            x += col_w + gap;
                            y = top;
                        }
                        push_item(&mut slots, idx, x, y, col_w, row_h);
                        y += row_h;
                    }
                }
                cw = (x + col_w + 12.0).max(w);
                ch = h;
            }
            Mode::Small | Mode::Medium | Mode::Large | Mode::Xl | Mode::Tiles => {
                let avail = (w - 24.0).max(60.0);
                let (n, tw, gx, gy) = if self.mode == Mode::Small {
                    let n = (((avail + 8.0) / 208.0).floor() as usize).max(1);
                    (n, (avail - (n as f64 - 1.0) * 8.0) / n as f64, 8.0, 0.0)
                } else {
                    let (tw, gap) = match self.mode {
                        Mode::Xl => (272.0, 4.0),
                        Mode::Large => (124.0, 2.0),
                        Mode::Tiles => (264.0, 4.0),
                        _ => (88.0, 2.0),
                    };
                    ((((avail + gap) / (tw + gap)).floor() as usize).max(1), tw, gap, gap)
                };
                let x0 = 12.0;
                let mut y = 8.0;
                for sec in sections.iter() {
                    if let Some(g) = sec.group {
                        y += 4.0;
                        slots.push(Slot { kind: SlotKind::Group(g), x: x0, y, w: avail, h: 36.0 });
                        y += 36.0 + gy;
                    }
                    let mut col = 0;
                    for idx in sec.start..sec.start + sec.len {
                        push_item(&mut slots, idx, x0 + col as f64 * (tw + gx), y, tw, row_h);
                        col += 1;
                        if col == n {
                            col = 0;
                            y += row_h + gy;
                        }
                    }
                    if col != 0 {
                        y += row_h + gy;
                    }
                }
                cw = w;
                ch = y + 8.0 + 16.0;
            }
        }
        let (mut cw, mut ch) = (cw, ch);
        if order_len == 0 && self.empty.is_some() {
            let top = self.top_inset();
            let ew = (w - 32.0).max(100.0);
            let eh = self.empty.as_ref().map(|e| e.measure(gtk::Orientation::Vertical, ew as i32).1 as f64).unwrap_or(40.0).max(20.0);
            slots.push(Slot { kind: SlotKind::Empty, x: 16.0, y: top + 20.0, w: ew, h: eh });
            ch = ch.max(top + 20.0 + eh + 20.0);
            cw = cw.max(w);
        }
        *self.slots.borrow_mut() = slots;
        *self.item_slot.borrow_mut() = item_slot;
        self.content_size.set((cw, ch));
    }

    /// The canvas got its size: lay out when needed, set the adjustments, place the widgets in view.
    fn allocate(self: &Rc<Self>, w: i32, h: i32) {
        if self.destroyed.get() {
            return;
        }
        self.view_size.set((w, h));
        let key = (w, if self.mode == Mode::List { h } else { 0 });
        let first = self.layout_key.get().0 < 0;
        if self.dirty.get() || key != self.layout_key.get() {
            self.layout(w, h);
            self.dirty.set(false);
            self.layout_key.set(key);
        }
        let (cw, ch) = self.content_size.get();
        let (hadj, vadj) = self.canvas.adjustments();
        self.canvas.imp().configuring.set(true);
        if let Some(a) = &hadj {
            let max = (cw - w as f64).max(0.0);
            a.configure(a.value().clamp(0.0, max), 0.0, cw.max(w as f64), 24.0, w as f64 * 0.9, w as f64);
        }
        if let Some(a) = &vadj {
            let max = (ch - h as f64).max(0.0);
            a.configure(a.value().clamp(0.0, max), 0.0, ch.max(h as f64), 32.0, h as f64 * 0.9, h as f64);
        }
        if let Some(pos) = self.pending_restore.take() {
            self.apply_scroll_now(pos, &hadj, &vadj);
        }
        let reveal = self.pending_reveal.borrow_mut().take();
        if let Some((path, focus)) = reveal {
            self.scroll_to_now(&path, &hadj, &vadj);
            if focus {
                let c = self.canvas.clone();
                glib::idle_add_local_once(move || {
                    c.grab_focus();
                });
            }
        }
        self.canvas.imp().configuring.set(false);
        self.place(w, h);
        if first {
            let cbs: Vec<Box<dyn FnOnce()>> = self.after_layout.borrow_mut().drain(..).collect();
            for f in cbs {
                glib::idle_add_local_once(f);
            }
        }
    }

    /// The scroll position changed (from the scroll bars or code): the rubber band follows.
    fn scrolled(self: &Rc<Self>) {
        if self.band.borrow().is_some() {
            band_update(self, None);
        }
    }

    fn apply_scroll(&self, pos: (f64, f64)) {
        let (h, v) = self.canvas.adjustments();
        self.apply_scroll_now(pos, &h, &v);
    }

    fn apply_scroll_now(&self, (top, left): (f64, f64), h: &Option<gtk::Adjustment>, v: &Option<gtk::Adjustment>) {
        if let Some(a) = v {
            a.set_value(top.clamp(0.0, (a.upper() - a.page_size()).max(0.0)));
        }
        if let Some(a) = h {
            a.set_value(left.clamp(0.0, (a.upper() - a.page_size()).max(0.0)));
        }
    }

    fn slot_of(&self, path: &str) -> Option<Slot> {
        let idx = *self.index.borrow().get(path)?;
        let si = *self.item_slot.borrow().get(idx)?;
        self.slots.borrow().get(si).copied()
    }

    fn scroll_to(&self, path: &str) {
        let (h, v) = self.canvas.adjustments();
        self.scroll_to_now(path, &h, &v);
    }

    /// Scroll so the item is in view ("nearest", with room for the column header).
    fn scroll_to_now(&self, path: &str, h: &Option<gtk::Adjustment>, v: &Option<gtk::Adjustment>) {
        let Some(s) = self.slot_of(path) else { return };
        let top_pad = if self.mode == Mode::Details { HEADER_H + 4.0 } else { 0.0 };
        if let Some(a) = v {
            let (val, page) = (a.value(), a.page_size());
            let target = if s.y - top_pad < val {
                s.y - top_pad
            } else if s.y + s.h > val + page {
                s.y + s.h - page
            } else {
                val
            };
            a.set_value(target.clamp(0.0, (a.upper() - page).max(0.0)));
        }
        // A Details row spans every column, so it is often wider than the view (the details pane open): revealing
        // it never scrolls sideways, or the Naam column went out of view. Elsewhere an item wider than the view
        // shows its start.
        if let Some(a) = h
            && self.mode != Mode::Details
        {
            let (val, page) = (a.value(), a.page_size());
            let target = if s.x < val || s.w > page {
                s.x - 8.0
            } else if s.x + s.w > val + page {
                s.x + s.w - page + 8.0
            } else {
                val
            };
            a.set_value(target.clamp(0.0, (a.upper() - page).max(0.0)));
        }
    }

    /// Give the slots in view a widget, take it away from the others, and allocate everything.
    fn place(self: &Rc<Self>, w: i32, h: i32) {
        let (hv, vv) = self.canvas.scroll_pos();
        let slots = self.slots.borrow();
        let list = self.mode == Mode::List;
        let (lo, hi) = if list { (hv - OVERSCAN, hv + w as f64 + OVERSCAN) } else { (vv - OVERSCAN, vv + h as f64 + OVERSCAN) };
        let start = slots.partition_point(|s| if list { s.x + s.w < lo } else { s.y + s.h < lo });
        let mut wanted: Vec<usize> = Vec::new();
        for (si, s) in slots.iter().enumerate().skip(start) {
            if (if list { s.x } else { s.y }) > hi {
                break;
            }
            wanted.push(si);
        }
        if let Some(e) = slots.iter().position(|s| s.kind == SlotKind::Empty)
            && !wanted.contains(&e)
        {
            wanted.push(e);
        }
        // the item being renamed keeps its widget
        let pinned = self.rename.borrow().as_ref().and_then(|r| {
            let idx = *self.index.borrow().get(&r.path)?;
            self.item_slot.borrow().get(idx).copied()
        });
        if let Some(p) = pinned
            && !wanted.contains(&p)
        {
            wanted.push(p);
        }
        let wanted_set: HashSet<usize> = wanted.iter().copied().collect();
        // release
        let gone: Vec<usize> = self.live.borrow().keys().filter(|k| !wanted_set.contains(k)).copied().collect();
        for si in gone {
            if let Some(l) = self.live.borrow_mut().remove(&si) {
                match l {
                    Live::Item(c) => {
                        c.root.set_child_visible(false);
                        self.pool.borrow_mut().push(c);
                    }
                    Live::Group(g) => {
                        g.root.set_child_visible(false);
                        self.gpool.borrow_mut().push(g);
                    }
                    Live::Empty => {
                        if let Some(e) = &self.empty {
                            e.set_child_visible(false);
                        }
                    }
                }
            }
        }
        // acquire and bind
        let order = self.order.borrow();
        for &si in &wanted {
            if self.live.borrow().contains_key(&si) {
                continue;
            }
            let live = match slots[si].kind {
                SlotKind::Item(idx) => {
                    let cell = self.pool.borrow_mut().pop().unwrap_or_else(|| self.make_cell());
                    if let Some(item) = order.get(idx) {
                        self.bind_cell(&cell, item);
                    }
                    cell.root.set_child_visible(true);
                    Live::Item(cell)
                }
                SlotKind::Group(g) => {
                    let row = self.gpool.borrow_mut().pop().unwrap_or_else(|| self.make_group_row());
                    self.bind_group(&row, g);
                    row.root.set_child_visible(true);
                    Live::Group(row)
                }
                SlotKind::Empty => {
                    if let Some(e) = &self.empty {
                        e.set_child_visible(true);
                    }
                    Live::Empty
                }
            };
            self.live.borrow_mut().insert(si, live);
        }
        drop(order);
        // allocate
        for (si, l) in self.live.borrow().iter() {
            let s = slots[*si];
            let widget: gtk::Widget = match l {
                Live::Item(c) => c.root.clone().upcast(),
                Live::Group(g) => g.root.clone().upcast(),
                Live::Empty => match &self.empty {
                    Some(e) => e.clone(),
                    None => continue,
                },
            };
            let mut sh = s.h;
            let (_, nat_w, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
            let _ = nat_w;
            let (min_h, nat_h, _, _) = widget.measure(gtk::Orientation::Vertical, s.w as i32);
            if pinned == Some(*si) {
                sh = sh.max(nat_h as f64);
            }
            sh = sh.max(min_h as f64);
            widget.size_allocate(
                &gtk::Allocation::new((s.x - hv).round() as i32, (s.y - vv).round() as i32, s.w.round() as i32, sh.round() as i32),
                -1,
            );
        }
        drop(slots);
        if let Some(head) = self.header.borrow().as_ref() {
            let (cw, _) = self.content_size.get();
            let hw = cw.max(w as f64).round() as i32;
            let _ = head.measure(gtk::Orientation::Horizontal, -1);
            let _ = head.measure(gtk::Orientation::Vertical, hw);
            head.size_allocate(&gtk::Allocation::new(-(hv.round() as i32), 0, hw, HEADER_H as i32), -1);
        }
        self.place_band();
        self.paint();
    }

    fn place_band(&self) {
        let b = self.band.borrow();
        let Some(band) = b.as_ref().filter(|b| b.moved) else {
            self.band_widget.set_child_visible(false);
            return;
        };
        let (hv, vv) = self.canvas.scroll_pos();
        let (px, py) = (band.last.0 + hv, band.last.1 + vv);
        let (x1, y1, x2, y2) = (band.x0.min(px), band.y0.min(py), band.x0.max(px), band.y0.max(py));
        self.band_widget.set_child_visible(true);
        let _ = self.band_widget.measure(gtk::Orientation::Horizontal, -1);
        let _ = self.band_widget.measure(gtk::Orientation::Vertical, -1);
        self.band_widget.size_allocate(
            &gtk::Allocation::new(
                (x1 - hv).round() as i32,
                (y1 - vv).round() as i32,
                ((x2 - x1).round() as i32).max(1),
                ((y2 - y1).round() as i32).max(1),
            ),
            -1,
        );
    }

    fn set_col_width(&self, ci: usize, w: f64) {
        if let Some(slot) = self.col_widths.borrow_mut().get_mut(ci) {
            *slot = w;
        }
        let px = w.round() as i32;
        if let Some(c) = self.header_cells.borrow().get(ci) {
            c.set_size_request(px, -1);
        }
        let box_index = ci;
        for c in self.all_cells.borrow().iter() {
            if let Some(b) = c.col_boxes.get(box_index) {
                b.set_size_request(px, -1);
            }
        }
        self.dirty.set(true);
        self.canvas.queue_allocate();
    }

    /// The width that fits the widest text of a column (first 400 items).
    fn fit_width(&self, ci: usize) -> f64 {
        let Some(col) = self.columns.get(ci) else { return 120.0 };
        let mut max = 60.0f64;
        for it in self.order.borrow().iter().take(400) {
            let text = if ci == 0 { display_name(it, self.show_ext) } else { (col.render)(it) };
            let layout = self.canvas.create_pango_layout(Some(&text));
            let (w, _) = layout.pixel_size();
            max = max.max(w as f64 + if ci == 0 { 40.0 } else { 24.0 });
        }
        max
    }

    fn make_cell(&self) -> Rc<ItemCell> {
        let mode = self.mode;
        let root = gtk::Overlay::new();
        root.add_css_class("xp-item");
        let content = gtk::Box::new(if mode.stacked() { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal }, 0);
        content.add_css_class("xp-item-content");
        root.set_child(Some(&content));
        let icon_px = mode.icon_size();
        let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon.add_css_class("xp-ficon");
        icon.set_size_request(icon_px, icon_px);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        let name_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        name_box.add_css_class("xp-namebox");
        let name = label("xp-item-name");
        name_box.append(&name);
        let mut subs = Vec::new();
        let mut col_boxes = Vec::new();
        let mut col_labels = Vec::new();
        let mut check = None;
        let inline_check = |content: &gtk::Box, width: i32| {
            let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            wrap.set_size_request(width, -1);
            wrap.set_hexpand(false);
            let c = check_box(12);
            c.set_hexpand(true);
            wrap.append(&c);
            content.append(&wrap);
            c
        };
        match mode {
            Mode::Details => {
                if self.checkboxes {
                    check = Some(inline_check(&content, CHECK_COL as i32));
                }
                let widths = self.col_widths.borrow();
                for (ci, col) in self.columns.iter().enumerate() {
                    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    b.add_css_class("xp-cell");
                    b.set_size_request(widths[ci].round() as i32, -1);
                    if ci == 0 {
                        b.add_css_class("xp-cell-name");
                        b.append(&icon);
                        name_box.set_hexpand(true);
                        b.append(&name_box);
                    } else {
                        let l = label("xp-cell-text");
                        l.set_hexpand(true);
                        if col.align_end {
                            l.set_xalign(1.0);
                            b.add_css_class("is-end");
                        }
                        b.append(&l);
                        col_labels.push(l);
                    }
                    content.append(&b);
                    col_boxes.push(b);
                }
            }
            Mode::List | Mode::Small => {
                content.set_spacing(8);
                if self.checkboxes {
                    let c = check_box(12);
                    content.append(&c);
                    check = Some(c);
                }
                content.append(&icon);
                name_box.set_hexpand(true);
                content.append(&name_box);
            }
            Mode::Tiles => {
                content.set_spacing(10);
                if self.checkboxes {
                    let c = check_box(12);
                    c.set_halign(gtk::Align::Start);
                    c.set_valign(gtk::Align::Start);
                    c.set_margin_start(4);
                    c.set_margin_top(4);
                    root.add_overlay(&c);
                    check = Some(c);
                }
                content.append(&icon);
                let meta = gtk::Box::new(gtk::Orientation::Vertical, 0);
                meta.add_css_class("xp-item-meta");
                meta.set_valign(gtk::Align::Center);
                meta.set_hexpand(true);
                meta.append(&name_box);
                for _ in 0..2 {
                    let l = label("xp-item-sub");
                    meta.append(&l);
                    subs.push(l);
                }
                content.append(&meta);
            }
            Mode::Content => {
                content.set_spacing(12);
                if self.checkboxes {
                    let c = check_box(12);
                    c.set_halign(gtk::Align::Start);
                    c.set_valign(gtk::Align::Start);
                    c.set_margin_top(4);
                    root.add_overlay(&c);
                    check = Some(c);
                }
                content.append(&icon);
                let meta = gtk::Box::new(gtk::Orientation::Vertical, 0);
                meta.add_css_class("xp-item-meta");
                meta.set_valign(gtk::Align::Center);
                meta.set_hexpand(true);
                name.add_css_class("xp-content-name");
                meta.append(&name_box);
                let l = label("xp-item-sub");
                meta.append(&l);
                subs.push(l);
                content.append(&meta);
                let props = gtk::Box::new(gtk::Orientation::Vertical, 0);
                props.add_css_class("xp-item-props");
                props.set_valign(gtk::Align::Center);
                props.set_size_request(200, -1);
                for _ in 0..2 {
                    let l = label("xp-item-sub");
                    props.append(&l);
                    subs.push(l);
                }
                content.append(&props);
            }
            Mode::Medium | Mode::Large | Mode::Xl => {
                content.set_spacing(if mode == Mode::Xl { 6 } else { 4 });
                if self.checkboxes {
                    let c = check_box(12);
                    c.set_halign(gtk::Align::Start);
                    c.set_valign(gtk::Align::Start);
                    c.set_margin_start(4);
                    c.set_margin_top(4);
                    root.add_overlay(&c);
                    check = Some(c);
                }
                content.append(&icon);
                name.set_single_line_mode(false);
                name.set_wrap(true);
                name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                name.set_lines(2);
                name.set_justify(gtk::Justification::Center);
                name.set_xalign(0.5);
                name.set_max_width_chars(1);
                name.set_hexpand(true);
                name_box.set_halign(gtk::Align::Fill);
                content.append(&name_box);
            }
        }
        root.set_parent(&self.canvas);
        let cell = Rc::new(ItemCell {
            root,
            icon,
            name_box,
            name,
            subs,
            col_boxes,
            col_labels,
            check,
            bound: RefCell::new(None),
        });
        self.all_cells.borrow_mut().push(cell.clone());
        cell
    }

    fn bind_cell(&self, cell: &ItemCell, item: &Item) {
        let path = st(item, "path").to_owned();
        if cell.bound.borrow().as_deref() == Some(path.as_str()) {
            return;
        }
        while let Some(c) = cell.icon.first_child() {
            cell.icon.remove(&c);
        }
        let px = self.mode.icon_size();
        let img = kit::icons::file_item_icon(item, px, px >= 32);
        img.set_halign(gtk::Align::Center);
        img.set_valign(gtk::Align::Center);
        cell.icon.append(&img);
        let name = display_name(item, self.show_ext);
        cell.name.set_label(&name);
        if self.mode.stacked() || self.mode == Mode::Tiles || self.mode == Mode::Content {
            cell.root.set_tooltip_text(Some(&name));
        }
        match self.mode {
            Mode::Details => {
                for (li, col) in self.columns.iter().skip(1).enumerate() {
                    if let Some(l) = cell.col_labels.get(li) {
                        l.set_label(&(col.render)(item));
                    }
                }
            }
            Mode::Tiles => {
                if let Some(l) = cell.subs.first() {
                    l.set_label(&(self.type_of)(item));
                }
                if let Some(l) = cell.subs.get(1) {
                    l.set_label(&if flag(item, "isDir") { String::new() } else { (self.size_of)(item) });
                }
            }
            Mode::Content => {
                let (a, b) = (self.content_props)(item);
                let texts = [(self.type_of)(item), a, b];
                for (l, t) in cell.subs.iter().zip(texts.iter()) {
                    l.set_label(t);
                }
            }
            _ => {}
        }
        let set = |c: &str, on: bool| {
            if on {
                cell.root.add_css_class(c);
            } else {
                cell.root.remove_css_class(c);
            }
        };
        set("is-hidden", flag(item, "hidden"));
        set("is-dir", flag(item, "isDir"));
        *cell.bound.borrow_mut() = Some(path);
    }

    fn make_group_row(&self) -> Rc<GroupRow> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("xp-group");
        let cb = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cb.add_css_class("xp-group-chev");
        cb.set_size_request(16, -1);
        cb.set_hexpand(false);
        let chev = kit::icons::glyph("chevron-down", 12);
        chev.set_hexpand(true);
        cb.append(&chev);
        let label = label("xp-group-label");
        label.set_hexpand(false);
        let count = label_plain("xp-group-count");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class("xp-group-line");
        line.set_hexpand(true);
        line.set_valign(gtk::Align::Center);
        line.set_size_request(-1, 1);
        root.append(&cb);
        root.append(&label);
        root.append(&count);
        root.append(&line);
        root.set_parent(&self.canvas);
        Rc::new(GroupRow { root, chev, label, count })
    }

    fn bind_group(&self, row: &GroupRow, gi: usize) {
        let Some(g) = self.groups.get(gi) else { return };
        let collapsed = self.collapsed.borrow().contains(&g.key);
        kit::icons::set_glyph(&row.chev, if collapsed { "chevron-right" } else { "chevron-down" });
        row.label.set_label(&g.label);
        row.count.set_label(&format!("({})", g.items.len()));
        row.root.set_tooltip_text(Some(s(if collapsed { "expandGroup" } else { "collapseGroup" })));
    }

    fn toggle_group(&self, gi: usize) {
        let Some(g) = self.groups.get(gi) else { return };
        {
            let mut c = self.collapsed.borrow_mut();
            if !c.remove(&g.key) {
                c.insert(g.key.clone());
            }
        }
        // every slot index changes: give all widgets back first
        let all: Vec<(usize, Live)> = self.live.borrow_mut().drain().collect();
        for (_, l) in all {
            match l {
                Live::Item(c) => {
                    c.root.set_child_visible(false);
                    self.pool.borrow_mut().push(c);
                }
                Live::Group(g) => {
                    g.root.set_child_visible(false);
                    self.gpool.borrow_mut().push(g);
                }
                Live::Empty => {}
            }
        }
        // a selection inside a collapsed group is dropped from the visible order only
        self.rebuild_order();
        self.canvas.queue_allocate();
    }

    /// Selection, focus, cut, drag and drop classes of the widgets in view.
    fn paint(&self) {
        let sel = self.selection.borrow();
        let cut = self.cut.borrow();
        let dragging = self.dragging.borrow();
        let drop = self.drop_mark.borrow();
        let focus = self.focus.borrow();
        for l in self.live.borrow().values() {
            if let Live::Item(c) = l {
                let Some(p) = c.bound.borrow().clone() else { continue };
                let set = |cls: &str, on: bool| {
                    if on {
                        c.root.add_css_class(cls);
                    } else {
                        c.root.remove_css_class(cls);
                    }
                };
                set("is-selected", sel.contains(&p));
                set("is-cut", cut.contains(&p));
                set("is-dragging", dragging.contains(&p));
                set("is-drop", drop.as_deref() == Some(p.as_str()));
                set("is-focus", focus.as_deref() == Some(p.as_str()));
            }
        }
        if sel.is_empty() {
            self.root.remove_css_class("has-selection");
        } else {
            self.root.add_css_class("has-selection");
        }
        if let Some(hc) = self.header_check.borrow().as_ref() {
            if !sel.is_empty() && sel.len() == self.items.len() {
                hc.add_css_class("is-on");
            } else {
                hc.remove_css_class("is-on");
            }
        }
    }

    fn emit(&self) {
        self.paint();
        let sel = self.selection.borrow().clone();
        let f = self.focus.borrow().clone();
        (self.h.on_select)(&sel, f);
    }

    fn select(&self, paths: impl Iterator<Item = String>, focus: Option<String>, scroll: bool, silent: bool) {
        {
            let mut sel = self.selection.borrow_mut();
            sel.clear();
            for p in paths {
                if self.by_path.contains_key(&p) {
                    sel.insert(p);
                }
            }
        }
        if let Some(f) = focus.filter(|f| self.by_path.contains_key(f)) {
            self.set_roving(&f, scroll);
            *self.anchor.borrow_mut() = Some(f);
        }
        if silent {
            self.paint();
        } else {
            self.emit();
        }
    }

    fn set_roving(&self, path: &str, scroll: bool) {
        *self.focus.borrow_mut() = Some(path.to_owned());
        if scroll {
            self.scroll_to(path);
        }
        self.paint();
    }

    fn select_range(&self, from: &str, to: &str, add: bool) {
        let index = self.index.borrow();
        let Some(&b) = index.get(to) else { return };
        let (lo, hi) = match index.get(from) {
            Some(&a) => (a.min(b), a.max(b)),
            None => (b, b),
        };
        let order = self.order.borrow();
        let mut sel = self.selection.borrow_mut();
        if !add {
            sel.clear();
        }
        for it in &order[lo..=hi.min(order.len().saturating_sub(1))] {
            sel.insert(st(it, "path").to_owned());
        }
    }

    /// What is at (x, y) in canvas coordinates; None on empty space and the column header.
    fn hit(&self, x: f64, y: f64) -> Option<SlotKind> {
        if self.mode == Mode::Details && y < HEADER_H {
            return None;
        }
        let (hv, vv) = self.canvas.scroll_pos();
        let (cx, cy) = (x + hv, y + vv);
        let slots = self.slots.borrow();
        for si in self.live.borrow().keys() {
            if let Some(s) = slots.get(*si)
                && s.kind != SlotKind::Empty
                && s.contains(cx, cy)
            {
                return Some(s.kind);
            }
        }
        None
    }

    fn item_of(&self, idx: usize) -> Option<Item> {
        self.order.borrow().get(idx).cloned()
    }

    fn rect_of(&self, path: &str) -> Option<Slot> {
        self.slot_of(path)
    }

    /// The neighbour of an item in a direction (fileview.js neighbor).
    fn neighbor(&self, path: &str, dir: &str) -> Option<String> {
        let order = self.order.borrow();
        let Some(&idx) = self.index.borrow().get(path) else { return order.first().map(|x| st(x, "path").to_owned()) };
        if self.mode.order_nav() && (dir == "down" || dir == "up") {
            let j = if dir == "down" { idx.checked_add(1)? } else { idx.checked_sub(1)? };
            return order.get(j).map(|x| st(x, "path").to_owned());
        }
        if self.mode != Mode::List && self.mode.order_nav() {
            return None;
        }
        let r0 = self.rect_of(path)?;
        let (c0x, c0y) = (r0.x + r0.w / 2.0, r0.y + r0.h / 2.0);
        let slots = self.slots.borrow();
        let item_slot = self.item_slot.borrow();
        let mut best: Option<usize> = None;
        let (mut best_a, mut best_b) = (f64::INFINITY, f64::INFINITY);
        for (j, _) in order.iter().enumerate() {
            if j == idx {
                continue;
            }
            let Some(r) = item_slot.get(j).and_then(|si| slots.get(*si)) else { continue };
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let (mut a, b) = match dir {
                "down" if cy > c0y + 2.0 => (cy - c0y, (cx - c0x).abs()),
                "up" if cy < c0y - 2.0 => (c0y - cy, (cx - c0x).abs()),
                "right" if cx > c0x + 2.0 => (cx - c0x, (cy - c0y).abs()),
                "left" if cx < c0x - 2.0 => (c0x - cx, (cy - c0y).abs()),
                _ => continue,
            };
            if self.mode != Mode::List && (dir == "left" || dir == "right") && b > 2.0 {
                continue;
            }
            if self.mode != Mode::List && (dir == "up" || dir == "down") {
                a = (a / 4.0).round();
            }
            if a < best_a - 0.5 || ((a - best_a).abs() <= 0.5 && b < best_b) {
                best = Some(j);
                best_a = a;
                best_b = b;
            }
        }
        best.and_then(|j| order.get(j)).map(|x| st(x, "path").to_owned())
    }

    fn move_to(&self, path: Option<String>, state: gdk::ModifierType) {
        let Some(path) = path else { return };
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        if shift {
            let from = self.anchor.borrow().clone().or_else(|| self.focus.borrow().clone()).unwrap_or_else(|| path.clone());
            self.select_range(&from, &path, ctrl);
            self.set_roving(&path, true);
            self.emit();
        } else if ctrl {
            self.set_roving(&path, true);
        } else {
            self.select(std::iter::once(path.clone()), Some(path), true, false);
        }
    }
}

fn label_plain(class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.add_css_class(class);
    l
}

// ------------------------------------------------------------------------------------------- pointer

fn on_press(i: &Rc<Inner>, g: &gtk::GestureClick, n: i32, x: f64, y: f64) {
    let button = g.current_button();
    let state = g.current_event_state();
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    // a press outside the rename box commits it
    if i.rename.borrow().is_some() {
        let inside = i.rename.borrow().as_ref().is_some_and(|r| {
            r.entry
                .compute_point(&i.canvas, &gtk::graphene::Point::new(0.0, 0.0))
                .map(|p| {
                    let (w, h) = (r.entry.width() as f64, r.entry.height() as f64);
                    x >= p.x() as f64 && x <= p.x() as f64 + w && y >= p.y() as f64 && y <= p.y() as f64 + h
                })
                .unwrap_or(false)
        });
        if inside {
            return;
        }
        commit_rename(i);
    }
    let hit = i.hit(x, y);
    if i.mode == Mode::Details && y < HEADER_H {
        return;
    }
    if let Some(SlotKind::Group(_)) = hit {
        return;
    }
    let item = match hit {
        Some(SlotKind::Item(idx)) => i.item_of(idx),
        _ => None,
    };
    let path = item.as_ref().map(|it| st(it, "path").to_owned());
    if button == gdk::BUTTON_MIDDLE {
        if let Some(it) = &item {
            g.set_state(gtk::EventSequenceState::Claimed);
            (i.h.on_middle)(it);
        }
        return;
    }
    if button == gdk::BUTTON_SECONDARY {
        g.set_state(gtk::EventSequenceState::Claimed);
        match &path {
            Some(p) if !i.selection.borrow().contains(p) => i.select(std::iter::once(p.clone()), Some(p.clone()), false, false),
            None if !ctrl && !i.selection.borrow().is_empty() => {
                let f = i.focus.borrow().clone();
                i.select(std::iter::empty(), f, false, false);
            }
            _ => {}
        }
        if let Some(p) = &path {
            i.set_roving(p, false);
        }
        i.canvas.grab_focus();
        (i.h.on_context)(x, y, item);
        return;
    }
    if button != gdk::BUTTON_PRIMARY {
        return;
    }
    *i.pending.borrow_mut() = None;
    let Some(p) = path else {
        // empty space: the rubber band (GestureDrag) takes it
        i.canvas.grab_focus();
        return;
    };
    let Some(item) = item else { return };
    i.canvas.grab_focus();
    if n == 2 {
        g.set_state(gtk::EventSequenceState::Claimed);
        (i.h.on_open)(&item);
        return;
    }
    // the check box toggles
    if i.checkboxes && on_check(i, &p, x, y) {
        {
            let mut sel = i.selection.borrow_mut();
            if !sel.remove(&p) {
                sel.insert(p.clone());
            }
        }
        *i.anchor.borrow_mut() = Some(p.clone());
        i.set_roving(&p, true);
        i.emit();
        return;
    }
    if shift {
        let from = i.anchor.borrow().clone().unwrap_or_else(|| p.clone());
        i.select_range(&from, &p, ctrl);
        i.set_roving(&p, true);
        i.emit();
    } else if ctrl {
        *i.pending.borrow_mut() = Some((true, p.clone()));
        i.set_roving(&p, true);
    } else if !i.selection.borrow().contains(&p) {
        i.select(std::iter::once(p.clone()), Some(p.clone()), false, false);
        i.set_roving(&p, true);
    } else {
        *i.pending.borrow_mut() = Some((false, p.clone()));
        i.set_roving(&p, true);
    }
}

fn on_check(i: &Inner, path: &str, x: f64, y: f64) -> bool {
    let idx = match i.index.borrow().get(path) {
        Some(n) => *n,
        None => return false,
    };
    let Some(si) = i.item_slot.borrow().get(idx).copied() else { return false };
    let live = i.live.borrow();
    let Some(Live::Item(c)) = live.get(&si) else { return false };
    let Some(check) = &c.check else { return false };
    match check.compute_point(&i.canvas, &gtk::graphene::Point::new(0.0, 0.0)) {
        Some(p) => {
            let (px, py) = (p.x() as f64, p.y() as f64);
            x >= px - 4.0 && x <= px + 20.0 && y >= py - 4.0 && y <= py + 20.0
        }
        None => false,
    }
}

fn on_release(i: &Rc<Inner>, g: &gtk::GestureClick, x: f64, y: f64) {
    if g.current_button() == gdk::BUTTON_PRIMARY
        && let Some(SlotKind::Group(gi)) = i.hit(x, y)
    {
        i.toggle_group(gi);
        return;
    }
    let Some((toggle, path)) = i.pending.borrow_mut().take() else { return };
    let here = match i.hit(x, y) {
        Some(SlotKind::Item(idx)) => i.item_of(idx).map(|it| st(&it, "path").to_owned()),
        _ => None,
    };
    if here.as_deref() != Some(path.as_str()) {
        return;
    }
    if toggle {
        {
            let mut sel = i.selection.borrow_mut();
            if !sel.remove(&path) {
                sel.insert(path.clone());
            }
        }
        *i.anchor.borrow_mut() = Some(path);
        i.emit();
    } else if i.selection.borrow().len() > 1 {
        i.select(std::iter::once(path.clone()), Some(path), false, false);
    }
}

// --------------------------------------------------------------------------------------- rubber band

fn band_begin(i: &Rc<Inner>, g: &gtk::GestureDrag, x: f64, y: f64) {
    if i.hit(x, y).is_some() || (i.mode == Mode::Details && y < HEADER_H) || i.rename.borrow().is_some() {
        g.set_state(gtk::EventSequenceState::Denied);
        return;
    }
    // not on the scroll bars' strip of an overlay scrollbar: the canvas covers it, but a press there is fine
    g.set_state(gtk::EventSequenceState::Claimed);
    i.canvas.grab_focus();
    let state = g.current_event_state();
    let keep = state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK);
    let base: HashSet<String> = if keep { i.selection.borrow().clone() } else { HashSet::new() };
    if base.is_empty() && !i.selection.borrow().is_empty() {
        i.selection.borrow_mut().clear();
        i.paint();
    }
    let (hv, vv) = i.canvas.scroll_pos();
    *i.band.borrow_mut() = Some(Band { x0: x + hv, y0: y + vv, start: (x, y), last: (x, y), base, moved: false, timer: None });
}

fn band_update(i: &Rc<Inner>, offset: Option<(f64, f64)>) {
    let (x1, y1, x2, y2, base) = {
        let mut b = i.band.borrow_mut();
        let Some(band) = b.as_mut() else { return };
        if let Some((dx, dy)) = offset {
            band.last = (band.start.0 + dx, band.start.1 + dy);
        }
        let (hv, vv) = i.canvas.scroll_pos();
        let (px, py) = (band.last.0 + hv, band.last.1 + vv);
        let r = (band.x0.min(px), band.y0.min(py), band.x0.max(px), band.y0.max(py));
        if !band.moved && (r.2 - r.0 > 3.0 || r.3 - r.1 > 3.0) {
            band.moved = true;
        }
        if !band.moved {
            return;
        }
        if band.timer.is_none() {
            let weak = Rc::downgrade(i);
            band.timer = Some(glib::timeout_add_local(Duration::from_millis(16), move || match weak.upgrade() {
                Some(i) => auto_scroll(&i),
                None => glib::ControlFlow::Break,
            }));
        }
        (r.0, r.1, r.2, r.3, band.base.clone())
    };
    {
        let mut sel = i.selection.borrow_mut();
        sel.clear();
        sel.extend(base.iter().cloned());
        let slots = i.slots.borrow();
        let order = i.order.borrow();
        for s in slots.iter() {
            if let SlotKind::Item(idx) = s.kind
                && s.x + s.w > x1
                && s.x < x2
                && s.y + s.h > y1
                && s.y < y2
                && let Some(it) = order.get(idx)
            {
                let p = st(it, "path");
                if base.contains(p) {
                    sel.remove(p);
                } else {
                    sel.insert(p.to_owned());
                }
            }
        }
    }
    i.place_band();
    i.paint();
}

fn auto_scroll(i: &Rc<Inner>) -> glib::ControlFlow {
    let last = match i.band.borrow().as_ref() {
        Some(b) => b.last,
        None => return glib::ControlFlow::Break,
    };
    let (w, h) = i.view_size.get();
    let (w, h) = (w as f64, h as f64);
    let edge = 28.0;
    let top = if i.mode == Mode::Details { HEADER_H } else { 0.0 };
    let (hadj, vadj) = i.canvas.adjustments();
    let mut moved = false;
    if let Some(a) = vadj {
        let dy = if last.1 < top + edge {
            -((top + edge - last.1) / 3.0).ceil() - 4.0
        } else if last.1 > h - edge {
            ((last.1 - h + edge) / 3.0).ceil() + 4.0
        } else {
            0.0
        };
        if dy != 0.0 {
            let v = (a.value() + dy).clamp(0.0, (a.upper() - a.page_size()).max(0.0));
            if v != a.value() {
                a.set_value(v);
                moved = true;
            }
        }
    }
    if i.mode == Mode::List
        && let Some(a) = hadj
    {
        let dx = if last.0 < edge {
            -12.0
        } else if last.0 > w - edge {
            12.0
        } else {
            0.0
        };
        if dx != 0.0 {
            let v = (a.value() + dx).clamp(0.0, (a.upper() - a.page_size()).max(0.0));
            if v != a.value() {
                a.set_value(v);
                moved = true;
            }
        }
    }
    if moved {
        band_update(i, None);
    }
    glib::ControlFlow::Continue
}

fn band_end(i: &Rc<Inner>) {
    let Some(mut band) = i.band.borrow_mut().take() else { return };
    if let Some(id) = band.timer.take() {
        id.remove();
    }
    i.band_widget.set_child_visible(false);
    if band.moved {
        let first = {
            let sel = i.selection.borrow();
            i.order.borrow().iter().map(|x| st(x, "path").to_owned()).find(|p| sel.contains(p))
        };
        if let Some(f) = first {
            *i.anchor.borrow_mut() = Some(f.clone());
            i.set_roving(&f, false);
        }
    }
    i.emit();
}

// ------------------------------------------------------------------------------------------- keyboard

fn on_key(i: &Rc<Inner>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
    if i.rename.borrow().is_some() {
        return glib::Propagation::Proceed;
    }
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.contains(gdk::ModifierType::ALT_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    // the menu key / Shift+F10 open the context menu at the focused item
    if key == gdk::Key::Menu || (key == gdk::Key::F10 && shift) {
        let focus = i.focus.borrow().clone();
        let (hv, vv) = i.canvas.scroll_pos();
        let sel_has = focus.as_ref().is_some_and(|f| i.selection.borrow().contains(f));
        match focus.as_ref().filter(|_| sel_has).and_then(|f| i.rect_of(f).map(|r| (f.clone(), r))) {
            Some((f, r)) => {
                let item = i.by_path.get(&f).cloned();
                (i.h.on_context)(r.x - hv + 24.0, r.y - vv + r.h.min(28.0), item);
            }
            None => (i.h.on_context)(24.0, i.top_inset() + 28.0, None),
        }
        return glib::Propagation::Stop;
    }
    let order_len = i.order.borrow().len();
    if order_len == 0 {
        return glib::Propagation::Proceed;
    }
    let first = i.order.borrow().first().map(|x| st(x, "path").to_owned());
    let cur = i.focus.borrow().clone().or_else(|| first.clone()).unwrap_or_default();
    let dir = match key {
        gdk::Key::Down | gdk::Key::KP_Down => Some("down"),
        gdk::Key::Up | gdk::Key::KP_Up => Some("up"),
        gdk::Key::Left | gdk::Key::KP_Left => Some("left"),
        gdk::Key::Right | gdk::Key::KP_Right => Some("right"),
        _ => None,
    };
    if let Some(d) = dir {
        if alt {
            return glib::Propagation::Proceed;
        }
        let has_focus_item = i.focus.borrow().is_some();
        if !has_focus_item || i.selection.borrow().is_empty() && !ctrl && !shift {
            i.move_to(Some(cur), state);
            return glib::Propagation::Stop;
        }
        let n = i.neighbor(&cur, d);
        i.move_to(n, state);
        return glib::Propagation::Stop;
    }
    match key {
        gdk::Key::Home | gdk::Key::End if !alt => {
            let p = if key == gdk::Key::Home { first } else { i.order.borrow().last().map(|x| st(x, "path").to_owned()) };
            i.move_to(p, state);
            return glib::Propagation::Stop;
        }
        gdk::Key::Page_Down | gdk::Key::Page_Up => {
            let row_h = i.rect_of(&cur).map(|r| r.h).unwrap_or(32.0).max(1.0);
            let steps = ((i.view_size.get().1 as f64 / row_h).floor() as i64 - 1).max(1);
            let d = if key == gdk::Key::Page_Down { "down" } else { "up" };
            let mut p = cur.clone();
            for _ in 0..steps {
                match i.neighbor(&p, d) {
                    Some(n) => p = n,
                    None => break,
                }
            }
            i.move_to(Some(p), state);
            return glib::Propagation::Stop;
        }
        gdk::Key::space if !alt => {
            if ctrl {
                {
                    let mut sel = i.selection.borrow_mut();
                    if !sel.remove(&cur) {
                        sel.insert(cur.clone());
                    }
                }
                *i.anchor.borrow_mut() = Some(cur);
                i.emit();
            } else if !i.selection.borrow().contains(&cur) {
                i.select(std::iter::once(cur.clone()), Some(cur), false, false);
            }
            return glib::Propagation::Stop;
        }
        _ => {}
    }
    if ctrl && !alt && !shift && (key == gdk::Key::a || key == gdk::Key::A) {
        let all: Vec<String> = i.order.borrow().iter().map(|x| st(x, "path").to_owned()).collect();
        let f = i.focus.borrow().clone();
        i.select(all.into_iter(), f, false, false);
        return glib::Propagation::Stop;
    }
    // type to select
    if !ctrl
        && !alt
        && let Some(c) = key.to_unicode().filter(|c| !c.is_control() && *c != ' ')
    {
        let ch: String = c.to_lowercase().collect();
        if let Some(id) = i.type_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(i);
        *i.type_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(1000), move || {
            if let Some(i) = weak.upgrade() {
                i.type_timer.borrow_mut().take();
                i.type_buf.borrow_mut().clear();
            }
        }));
        let same = {
            let b = i.type_buf.borrow();
            b.chars().count() == 1 && *b == ch
        };
        if !same {
            i.type_buf.borrow_mut().push_str(&ch);
        }
        let buf = i.type_buf.borrow().clone();
        let start = i.index.borrow().get(&cur).copied().unwrap_or(0);
        let skip = if same || buf.chars().count() == 1 { 1 } else { 0 };
        let found = {
            let order = i.order.borrow();
            let n = order.len();
            (skip..=n)
                .map(|k| &order[(start + k) % n])
                .find(|it| display_name(it, i.show_ext).to_lowercase().starts_with(&buf))
                .map(|it| st(it, "path").to_owned())
        };
        if let Some(p) = found {
            i.select(std::iter::once(p.clone()), Some(p), true, false);
            return glib::Propagation::Stop;
        }
        return glib::Propagation::Proceed;
    }
    glib::Propagation::Proceed
}

// --------------------------------------------------------------------------------------------- rename

fn validate(v: &str) -> Option<&'static str> {
    let t = v.trim();
    if t.is_empty() {
        return Some(s("nameEmpty"));
    }
    if t.contains('/') {
        return Some(s("nameSlash"));
    }
    if t == "." || t == ".." {
        return Some(s("nameBad"));
    }
    None
}

fn start_rename(i: &Rc<Inner>, path: &str) -> bool {
    let Some(item) = i.by_path.get(path).cloned() else { return false };
    if !i.index.borrow().contains_key(path) {
        return false;
    }
    if i.rename.borrow().is_some() {
        cancel_rename(i);
    }
    i.select(std::iter::once(path.to_owned()), Some(path.to_owned()), true, false);
    let full = match st(&item, "name") {
        "" => crate::util::basename(path),
        n => n.to_owned(),
    };
    let (stem, ext) = split_ext(&full);
    let is_dir = flag(&item, "isDir");
    let hide_ext = !i.show_ext && !is_dir && !ext.is_empty() && !stem.is_empty();
    let value = if hide_ext { stem.clone() } else { full.clone() };
    let sel_end = if is_dir || hide_ext || ext.is_empty() || stem.is_empty() { value.chars().count() } else { stem.chars().count() };
    let multi = i.mode.stacked();
    let entry: gtk::Widget = if multi {
        let tv = gtk::TextView::new();
        tv.add_css_class("xp-rename-input");
        tv.add_css_class("is-multi");
        tv.set_wrap_mode(gtk::WrapMode::WordChar);
        tv.set_justification(gtk::Justification::Center);
        tv.set_accepts_tab(false);
        tv.buffer().set_text(&value);
        tv.upcast()
    } else {
        let e = gtk::Text::new();
        e.add_css_class("xp-rename-input");
        e.set_text(&value);
        e.set_hexpand(true);
        e.upcast()
    };
    entry.set_widget_name("explorer-rename");
    entry.update_property(&[gtk::accessible::Property::Label(s("renameMenu"))]);
    let tip_label = gtk::Label::new(None);
    tip_label.set_wrap(true);
    tip_label.set_max_width_chars(36);
    tip_label.set_xalign(0.0);
    let tip = gtk::Popover::new();
    tip.add_css_class("xp-rename-tip");
    tip.set_has_arrow(false);
    tip.set_autohide(false);
    tip.set_can_focus(false);
    tip.set_position(gtk::PositionType::Bottom);
    tip.set_child(Some(&tip_label));
    tip.set_parent(&entry);
    *i.rename.borrow_mut() = Some(Rename {
        path: path.to_owned(),
        item,
        entry: entry.clone(),
        tip,
        tip_label,
        hide_ext,
        ext,
        busy: false,
    });
    // give the item its widget now, then put the box in its name slot
    let (w, h) = i.view_size.get();
    i.place(w.max(1), h.max(1));
    let cell = i.live_cell(path);
    let Some(cell) = cell else {
        cancel_rename(i);
        return false;
    };
    cell.name.set_visible(false);
    cell.name_box.append(&entry);
    entry.set_hexpand(true);
    // keys (capture: before the text widget's own Enter handling)
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(i);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(i) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                commit_rename(&i);
                glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                cancel_rename(&i);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    entry.add_controller(keys);
    // validation balloon while typing
    let weak = Rc::downgrade(i);
    let on_change = move || {
        if let Some(i) = weak.upgrade() {
            let v = rename_text(&i);
            show_tip(&i, v.as_deref().and_then(validate));
        }
    };
    if let Some(t) = entry.downcast_ref::<gtk::Text>() {
        let f = on_change.clone();
        t.connect_changed(move |_| f());
    } else if let Some(tv) = entry.downcast_ref::<gtk::TextView>() {
        let f = on_change.clone();
        tv.buffer().connect_changed(move |_| f());
    }
    // leaving the box commits
    let focus = gtk::EventControllerFocus::new();
    let weak = Rc::downgrade(i);
    let ew = entry.downgrade();
    focus.connect_leave(move |_| {
        let weak = weak.clone();
        let ew = ew.clone();
        glib::idle_add_local_once(move || {
            if let (Some(i), Some(e)) = (weak.upgrade(), ew.upgrade()) {
                let same = i.rename.borrow().as_ref().is_some_and(|r| r.entry == e);
                if same && !e.has_focus() && !i.rename.borrow().as_ref().is_some_and(|r| r.busy) {
                    commit_rename(&i);
                }
            }
        });
    });
    entry.add_controller(focus);
    let e2 = entry.clone();
    glib::idle_add_local_once(move || {
        e2.grab_focus();
        if let Some(t) = e2.downcast_ref::<gtk::Text>() {
            t.select_region(0, sel_end as i32);
        } else if let Some(tv) = e2.downcast_ref::<gtk::TextView>() {
            let b = tv.buffer();
            let s = b.start_iter();
            let e = b.iter_at_offset(sel_end as i32);
            b.select_range(&s, &e);
        }
    });
    i.canvas.queue_allocate();
    true
}

impl Inner {
    fn live_cell(&self, path: &str) -> Option<Rc<ItemCell>> {
        let idx = *self.index.borrow().get(path)?;
        let si = *self.item_slot.borrow().get(idx)?;
        match self.live.borrow().get(&si) {
            Some(Live::Item(c)) => Some(c.clone()),
            _ => None,
        }
    }
}

fn rename_text(i: &Inner) -> Option<String> {
    let r = i.rename.borrow();
    let r = r.as_ref()?;
    if let Some(t) = r.entry.downcast_ref::<gtk::Text>() {
        return Some(t.text().to_string());
    }
    let tv = r.entry.downcast_ref::<gtk::TextView>()?;
    let b = tv.buffer();
    Some(b.text(&b.start_iter(), &b.end_iter(), false).to_string())
}

fn show_tip(i: &Inner, msg: Option<&str>) {
    let r = i.rename.borrow();
    let Some(r) = r.as_ref() else { return };
    match msg.filter(|m| !m.is_empty()) {
        Some(m) => {
            r.tip_label.set_label(m);
            r.tip.popup();
        }
        None => r.tip.popdown(),
    }
}

fn commit_rename(i: &Rc<Inner>) {
    let (item, hide_ext, ext) = {
        let r = i.rename.borrow();
        let Some(r) = r.as_ref() else { return };
        if r.busy {
            return;
        }
        (r.item.clone(), r.hide_ext, r.ext.clone())
    };
    let raw = rename_text(i).unwrap_or_default().trim().to_owned();
    let new_name = if hide_ext { format!("{raw}{ext}") } else { raw.clone() };
    let old = st(&item, "name").to_owned();
    if raw.is_empty() || new_name == old {
        cancel_rename(i);
        return;
    }
    if let Some(err) = validate(&raw) {
        show_tip(i, Some(err));
        if let Some(r) = i.rename.borrow().as_ref() {
            r.entry.grab_focus();
        }
        return;
    }
    if let Some(r) = i.rename.borrow_mut().as_mut() {
        r.busy = true;
    }
    let weak = Rc::downgrade(i);
    let nn = new_name.clone();
    let done: RenameDone = Box::new(move |res| {
        let Some(i) = weak.upgrade() else { return };
        if let Some(r) = i.rename.borrow_mut().as_mut() {
            r.busy = false;
        }
        match res {
            Ok(()) => finish_rename(&i, Some(&nn)),
            Err(msg) => {
                show_tip(&i, Some(if msg.is_empty() { s("errTitle") } else { &msg }));
                if let Some(r) = i.rename.borrow().as_ref() {
                    r.entry.grab_focus();
                }
            }
        }
    });
    (i.h.on_rename)(&item, &new_name, done);
}

fn finish_rename(i: &Rc<Inner>, new_name: Option<&str>) {
    let Some(r) = i.rename.borrow_mut().take() else { return };
    r.tip.popdown();
    r.tip.unparent();
    if let Some(cell) = i.live_cell(&r.path) {
        cell.name_box.remove(&r.entry);
        cell.name.set_visible(true);
        if let Some(n) = new_name {
            let mut v = (*r.item).clone();
            v["name"] = serde_json::Value::String(n.to_owned());
            v["displayName"] = serde_json::Value::Null;
            cell.name.set_label(&display_name(&v, i.show_ext));
        }
    } else if let Some(parent) = r.entry.parent().and_downcast::<gtk::Box>() {
        parent.remove(&r.entry);
    }
    i.canvas.grab_focus();
    i.canvas.queue_allocate();
    (i.h.on_rename_end)();
}

fn cancel_rename(i: &Rc<Inner>) {
    finish_rename(i, None);
}
