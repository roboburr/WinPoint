//! The result list as a lazy gio::ListModel (table.js's paging, natively): the backend says how many results
//! there are, page 0 comes with the search, and the other pages of 200 (index.search offset/limit) are fetched
//! when the ColumnView asks for one of their rows, so a million results cost no more than the rows on screen.
//! At most 80 pages are kept; rows whose page is not there yet are placeholders and redraw once it arrives
//! (the item's "changed" signal), as do all rows on a refresh that keeps the scroll position.
use gtk::glib;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::gio;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

/// Rows per page (table.js PAGE).
pub const PAGE: u32 = 200;
/// Pages kept in memory (table.js MAX_PAGES).
const MAX_PAGES: usize = 80;

/// One search result (engine.rs item()).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub is_dir: bool,
    pub size: f64,
    pub modified: f64,
    pub ext: String,
    pub mime: String,
    pub hidden: bool,
}

impl Entry {
    /// From the backend's JSON (missing fields stay empty; no path: not a result).
    pub fn from_value(v: &Value) -> Option<Entry> {
        let path = v["path"].as_str()?.to_owned();
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        let name = v["name"].as_str().map(str::to_owned).unwrap_or_else(|| {
            path.rsplit('/').find(|p| !p.is_empty()).unwrap_or(path.as_str()).to_owned()
        });
        Some(Entry {
            name,
            dir: s("dir"),
            is_dir: v["isDir"] == true,
            size: v["size"].as_f64().unwrap_or(0.0),
            modified: v["modified"].as_f64().unwrap_or(0.0),
            ext: s("ext"),
            mime: s("mime"),
            hidden: v["hidden"] == true,
            path,
        })
    }

    /// The folder it is in, "/" for the root.
    pub fn folder(&self) -> &str {
        if self.dir.is_empty() { "/" } else { &self.dir }
    }

    /// The MIME type for an icon ("inode/directory" for folders).
    pub fn icon_mime(&self) -> &str {
        if self.is_dir {
            "inode/directory"
        } else if self.mime.is_empty() {
            "application/octet-stream"
        } else {
            &self.mime
        }
    }
}

// ----------------------------------------------------------------------------------------------- the item

mod item_imp {
    use super::*;

    #[derive(Default)]
    pub struct Item {
        pub index: Cell<u32>,
        pub entry: RefCell<Option<Rc<Entry>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Item {
        const NAME: &'static str = "AllesItem";
        type Type = super::Item;
    }

    impl ObjectImpl for Item {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("changed").build()])
        }
    }
}

glib::wrapper! {
    /// One row of the result list: its index and, once its page is there, the result.
    pub struct Item(ObjectSubclass<item_imp::Item>);
}

impl Item {
    fn new(index: u32, entry: Option<Rc<Entry>>) -> Item {
        let it: Item = glib::Object::new();
        it.imp().index.set(index);
        *it.imp().entry.borrow_mut() = entry;
        it
    }

    pub fn index(&self) -> u32 {
        self.imp().index.get()
    }

    pub fn entry(&self) -> Option<Rc<Entry>> {
        self.imp().entry.borrow().clone()
    }

    fn set_entry(&self, e: Option<Rc<Entry>>) {
        *self.imp().entry.borrow_mut() = e;
        self.emit_by_name::<()>("changed", &[]);
    }

    /// Draw the row again (theme or highlight changed).
    pub fn refresh(&self) {
        self.emit_by_name::<()>("changed", &[]);
    }
}

// ------------------------------------------------------------------------------------------- the model

type Fetch = Rc<dyn Fn(u32, u64)>;

#[derive(Default)]
pub struct State {
    total: u32,
    gen_id: u64,
    pages: HashMap<u32, Rc<Vec<Rc<Entry>>>>,
    live: HashMap<u32, glib::WeakRef<Item>>,
    pending: HashSet<u32>,
    wanted: BTreeSet<u32>,
    timer: Option<glib::SourceId>,
    fetch: Option<Fetch>,
}

mod list_imp {
    use super::*;

    #[derive(Default)]
    pub struct Results {
        pub state: RefCell<State>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Results {
        const NAME: &'static str = "AllesResults";
        type Type = super::Results;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for Results {}

    impl ListModelImpl for Results {
        fn item_type(&self) -> glib::Type {
            Item::static_type()
        }

        fn n_items(&self) -> u32 {
            self.state.borrow().total
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let mut s = self.state.borrow_mut();
            if position >= s.total {
                return None;
            }
            if let Some(it) = s.live.get(&position).and_then(|w| w.upgrade()) {
                return Some(it.upcast());
            }
            let page = position / PAGE;
            let entry = s.pages.get(&page).and_then(|p| p.get((position % PAGE) as usize).cloned());
            let missing = entry.is_none();
            let it = Item::new(position, entry);
            if s.live.len() > 4096 {
                s.live.retain(|_, w| w.upgrade().is_some());
            }
            s.live.insert(position, it.downgrade());
            drop(s);
            if missing {
                self.obj().want(page);
            }
            Some(it.upcast())
        }
    }
}

glib::wrapper! {
    /// The result list.
    pub struct Results(ObjectSubclass<list_imp::Results>) @implements gio::ListModel;
}

impl Default for Results {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Results {
    /// `fetch(page, generation)` loads a page; its answer goes to `page_loaded`.
    pub fn set_fetch(&self, f: impl Fn(u32, u64) + 'static) {
        self.imp().state.borrow_mut().fetch = Some(Rc::new(f));
    }

    pub fn total(&self) -> u32 {
        self.imp().state.borrow().total
    }

    /// The generation of the result set (pages fetched for an older one are dropped).
    pub fn generation(&self) -> u64 {
        self.imp().state.borrow().gen_id
    }

    /// A result by index, if its page is there.
    pub fn entry(&self, index: u32) -> Option<Rc<Entry>> {
        let s = self.imp().state.borrow();
        s.pages.get(&(index / PAGE)).and_then(|p| p.get((index % PAGE) as usize).cloned())
    }

    /// Whether the page of this index is there.
    pub fn has_page(&self, page: u32) -> bool {
        self.imp().state.borrow().pages.contains_key(&page)
    }

    /// A new result set: `total` results, `first` = page 0. `keep`: the rows stay (scroll position and
    /// selection), they are only drawn again; otherwise every row is replaced.
    pub fn reset(&self, total: u32, first: Vec<Rc<Entry>>, keep: bool) {
        let (old, refresh) = {
            let mut s = self.imp().state.borrow_mut();
            s.gen_id += 1;
            s.pages.clear();
            s.pending.clear();
            s.wanted.clear();
            if let Some(t) = s.timer.take() {
                t.remove();
            }
            if !first.is_empty() {
                s.pages.insert(0, Rc::new(first));
            }
            let old = s.total;
            s.total = total;
            let mut refresh = Vec::new();
            if keep {
                s.live.retain(|i, w| *i < total && w.upgrade().is_some());
                for w in s.live.values() {
                    if let Some(it) = w.upgrade() {
                        refresh.push(it);
                    }
                }
            } else {
                s.live.clear();
            }
            (old, refresh)
        };
        if keep {
            let common = old.min(total);
            if old != total {
                self.items_changed(common, old - common, total - common);
            }
            for it in refresh {
                let idx = it.index();
                let e = self.entry(idx);
                let missing = e.is_none();
                it.set_entry(e);
                if missing {
                    self.want(idx / PAGE);
                }
            }
        } else {
            self.items_changed(0, old, total);
        }
    }

    /// Draw every row on screen again.
    pub fn refresh_all(&self) {
        let items: Vec<Item> = self.imp().state.borrow().live.values().filter_map(|w| w.upgrade()).collect();
        for it in items {
            it.refresh();
        }
    }

    fn want(&self, page: u32) {
        let mut s = self.imp().state.borrow_mut();
        if s.pages.contains_key(&page) || s.pending.contains(&page) || page * PAGE >= s.total {
            return;
        }
        s.wanted.insert(page);
        if s.timer.is_some() {
            return;
        }
        // A short pause: while scrolling fast the rows pass by before their page would arrive (table.js).
        let weak = self.downgrade();
        s.timer = Some(glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(me) = weak.upgrade() {
                me.imp().state.borrow_mut().timer = None;
                me.load_wanted();
            }
        }));
    }

    fn load_wanted(&self) {
        let (todo, fetch, gen_id) = {
            let mut s = self.imp().state.borrow_mut();
            let wanted: Vec<u32> = std::mem::take(&mut s.wanted).into_iter().collect();
            let mut todo = Vec::new();
            for p in wanted {
                let start = p * PAGE;
                let end = (start + PAGE).min(s.total);
                let shown = (start..end).any(|i| s.live.get(&i).is_some_and(|w| w.upgrade().is_some()));
                if shown && !s.pages.contains_key(&p) && !s.pending.contains(&p) {
                    s.pending.insert(p);
                    todo.push(p);
                }
            }
            (todo, s.fetch.clone(), s.gen_id)
        };
        if let Some(f) = fetch {
            for p in todo {
                f(p, gen_id);
            }
        }
    }

    /// A page arrived (or failed: `entries` None). Rows of it on screen are drawn again.
    pub fn page_loaded(&self, gen_id: u64, page: u32, entries: Option<Vec<Rc<Entry>>>) {
        let items = {
            let mut s = self.imp().state.borrow_mut();
            if s.gen_id != gen_id {
                return;
            }
            s.pending.remove(&page);
            let Some(entries) = entries else { return };
            s.pages.insert(page, Rc::new(entries));
            if s.pages.len() > MAX_PAGES {
                let live_pages: HashSet<u32> =
                    s.live.iter().filter(|(_, w)| w.upgrade().is_some()).map(|(i, _)| i / PAGE).collect();
                let mut far: Vec<u32> = s.pages.keys().copied().filter(|p| *p != page && *p != 0 && !live_pages.contains(p)).collect();
                far.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(page)));
                let excess = s.pages.len() - MAX_PAGES;
                for p in far.into_iter().take(excess) {
                    s.pages.remove(&p);
                }
            }
            let start = page * PAGE;
            let end = (start + PAGE).min(s.total);
            (start..end).filter_map(|i| s.live.get(&i).and_then(|w| w.upgrade())).collect::<Vec<Item>>()
        };
        for it in items {
            let e = self.entry(it.index());
            it.set_entry(e);
        }
    }

    /// Store a page fetched by someone else (copy, trash) for the current generation.
    pub fn store_page(&self, gen_id: u64, page: u32, entries: Vec<Rc<Entry>>) {
        if self.generation() == gen_id && !self.has_page(page) {
            self.page_loaded(gen_id, page, Some(entries));
        }
    }
}

/// The entries of a page answer (index.search {items}); a broken item keeps its place as an empty row.
pub fn entries_of(result: &Value) -> Vec<Rc<Entry>> {
    result["items"]
        .as_array()
        .map(|a| a.iter().map(|v| Rc::new(Entry::from_value(v).unwrap_or_default())).collect())
        .unwrap_or_default()
}
