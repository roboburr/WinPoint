//! Locations and tabs (main.js `loc` objects and `newTab`). A location is one of Start, Galerij, Deze pc,
//! Prullenbak, a folder or a search in a folder; a tab has its own history, listing, selection, filter and
//! remembered scroll positions. `TabState` is the serializable part (tabtransfer.js normalizeTabState shape,
//! version 1) that moves between windows and into the saved session.
use crate::util::Item;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Where a tab is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    Home,
    Gallery,
    ThisPc,
    Trash,
    Folder(String),
    Search { path: String, query: String },
}

impl Loc {
    /// main.js loc.kind: "home", "gallery", "thispc", "trash", "folder", "search".
    pub fn kind(&self) -> &'static str {
        match self {
            Loc::Home => "home",
            Loc::Gallery => "gallery",
            Loc::ThisPc => "thispc",
            Loc::Trash => "trash",
            Loc::Folder(_) => "folder",
            Loc::Search { .. } => "search",
        }
    }

    /// main.js locKey: "folder:/x", "search:/x:q", "home", ...
    pub fn key(&self) -> String {
        match self {
            Loc::Folder(p) => format!("folder:{p}"),
            Loc::Search { path, query } => format!("search:{path}:{query}"),
            other => other.kind().to_owned(),
        }
    }

    /// The folder of a folder or search location.
    pub fn path(&self) -> Option<&str> {
        match self {
            Loc::Folder(p) | Loc::Search { path: p, .. } => Some(p),
            _ => None,
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, Loc::Folder(_))
    }

    /// A folder or search result list ("fileish" in main.js).
    pub fn is_fileish(&self) -> bool {
        matches!(self, Loc::Folder(_) | Loc::Search { .. })
    }

    /// A location with a file listing (folder, Prullenbak, search).
    pub fn has_listing(&self) -> bool {
        matches!(self, Loc::Folder(_) | Loc::Trash | Loc::Search { .. })
    }

    /// {kind, path?, query?} as main.js keeps it.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn to_json(&self) -> Value {
        match self {
            Loc::Folder(p) => json!({ "kind": "folder", "path": p }),
            Loc::Search { path, query } => json!({ "kind": "search", "path": path, "query": query }),
            other => json!({ "kind": other.kind() }),
        }
    }

    /// Read {kind, path?, query?}; None for anything invalid (paths must be absolute).
    pub fn from_json(v: &Value) -> Option<Loc> {
        let kind = v.get("kind")?.as_str()?;
        let path = || v.get("path").and_then(Value::as_str).filter(|p| p.starts_with('/') && p.len() <= 16384).map(str::to_owned);
        Some(match kind {
            "home" => Loc::Home,
            "gallery" => Loc::Gallery,
            "thispc" => Loc::ThisPc,
            "trash" => Loc::Trash,
            "folder" => Loc::Folder(path()?),
            "search" => Loc::Search {
                path: path()?,
                query: v.get("query").and_then(Value::as_str).filter(|q| q.len() <= 4096)?.to_owned(),
            },
            _ => return None,
        })
    }
}

/// A tab's loading state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Loading,
    Ready,
    Error,
}

/// One tab (main.js newTab).
pub struct Tab {
    pub id: u64,
    pub loc: Loc,
    pub history: Vec<Loc>,
    pub h_index: usize,
    /// The listing (folder, Prullenbak, search); None while loading or for pages.
    pub items: Option<Vec<Item>>,
    pub by_path: HashMap<String, Item>,
    pub status: Status,
    pub error: Option<String>,
    pub selection: HashSet<String>,
    pub focus: Option<String>,
    pub filter: String,
    /// Bumped by every load; an answer for an older generation is dropped.
    pub generation: u64,
    /// Scroll position per location key (top, left).
    pub scroll_memo: HashMap<String, (f64, f64)>,
    pub pending_select: Option<Vec<String>>,
    pub pending_props: bool,
    pub pending_reveal: Option<String>,
    pub pending_props_items: Option<Vec<String>>,
    pub pending_rename: Option<String>,
    pub list_path: Option<String>,
    pub truncated: bool,
    pub want_focus: bool,
    pub reload_after_rename: bool,
    /// The Start tab a window opened with for a tab ticket: an imported tab replaces it.
    pub placeholder: bool,
}

impl Tab {
    pub fn new(id: u64, loc: Loc) -> Tab {
        Tab {
            id,
            history: vec![loc.clone()],
            loc,
            h_index: 0,
            items: None,
            by_path: HashMap::new(),
            status: Status::Idle,
            error: None,
            selection: HashSet::new(),
            focus: None,
            filter: String::new(),
            generation: 0,
            scroll_memo: HashMap::new(),
            pending_select: None,
            pending_props: false,
            pending_reveal: None,
            pending_props_items: None,
            pending_rename: None,
            list_path: None,
            truncated: false,
            want_focus: false,
            reload_after_rename: false,
            placeholder: false,
        }
    }

    /// The tab's semantic state (tabtransfer.js normalizeTabState shape): never listings or requests.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn state(&self) -> Value {
        let mut sel: Vec<&String> = self.selection.iter().collect();
        sel.sort();
        json!({
            "version": 1,
            "loc": self.loc.to_json(),
            "history": self.history.iter().map(Loc::to_json).collect::<Vec<_>>(),
            "hIndex": self.h_index,
            "selection": sel,
            "focus": self.focus,
            "filter": self.filter,
            "scrollMemo": self.scroll_memo.iter().map(|(k, (t, l))| json!([k, { "top": t, "left": l }])).collect::<Vec<_>>(),
            "pendingSelect": self.pending_select,
            "pendingProps": self.pending_props,
            "pendingReveal": self.pending_reveal,
        })
    }

    /// A tab from a state made by `state()` (or a validated one from another window); None when the
    /// location is invalid. Lenient about the rest.
    pub fn from_state(id: u64, v: &Value) -> Option<Tab> {
        let loc = Loc::from_json(v.get("loc")?)?;
        let mut tab = Tab::new(id, loc.clone());
        let history: Vec<Loc> = v["history"].as_array().map(|a| a.iter().filter_map(Loc::from_json).take(60).collect()).unwrap_or_default();
        let h_index = v["hIndex"].as_u64().unwrap_or(0) as usize;
        if !history.is_empty() && h_index < history.len() && history[h_index] == loc {
            tab.history = history;
            tab.h_index = h_index;
        }
        let paths = |x: &Value| -> Vec<String> {
            match x {
                Value::String(s) if s.starts_with('/') => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).filter(|s| s.starts_with('/')).map(str::to_owned).collect(),
                _ => Vec::new(),
            }
        };
        tab.selection = paths(&v["selection"]).into_iter().collect();
        tab.focus = v["focus"].as_str().filter(|s| s.starts_with('/')).map(str::to_owned);
        tab.filter = v["filter"].as_str().unwrap_or("").chars().take(4096).collect();
        if let Some(memo) = v["scrollMemo"].as_array() {
            for e in memo.iter().take(120) {
                if let (Some(k), Some(t), Some(l)) = (e[0].as_str(), e[1]["top"].as_f64(), e[1]["left"].as_f64()) {
                    tab.scroll_memo.insert(k.to_owned(), (t.max(0.0), l.max(0.0)));
                }
            }
        }
        let ps = paths(&v["pendingSelect"]);
        tab.pending_select = if ps.is_empty() { None } else { Some(ps) };
        tab.pending_props = v["pendingProps"] == true;
        tab.pending_reveal = v["pendingReveal"].as_str().filter(|s| s.starts_with('/')).map(str::to_owned);
        Some(tab)
    }
}
