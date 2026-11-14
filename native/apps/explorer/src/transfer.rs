//! Tabs between windows and the saved session (vendor/ui/js/apps/explorer/tabtransfer.js, tabs.js and the
//! session part of main.js, host/src/window_state.rs).
//!
//! # Moving a tab
//!
//! A tab is a drag source (`application/x-win11-explorer-tab+json`, an envelope {ticket, sourceWindow}). All
//! Explorer windows live in this one process, so a tab moves between them in-process; the transfer keeps the
//! WebKit app's rules:
//! - dropped on the window's own tab strip: it moves there (reorder);
//! - dropped on another window's tab strip: that window imports it before the tab under the pointer and loads
//!   it; only then the source closes its tab, and only when the tab did not change meanwhile ("tabKeptChanged");
//! - dropped outside the Explorer windows (the desktop, another app) or on another Explorer window's content: a
//!   new window opens with the tab (the window gets a local ticket through `--tab-ticket`'s path,
//!   `claim_ticket`, and replaces its placeholder Start tab);
//! - dropped on the source window's own content, or cancelled with Escape: nothing happens.
//!
//! GTK 4 tells the source whether one of the windows took the drop; every Explorer window catches tab drops on
//! its whole surface for that. A drag that nobody took is a tear-off unless it was cancelled: GTK says so on X11,
//! on Wayland the extension does (`explorer.tabPosition` {pid, phase "begin" | "end"}: its Escape bit and the
//! pointer at the end; without the extension a cancel over one of the windows counts as Escape).
//!
//! Tabs of another process (a WebKit Explorer, its plain-text envelope `win11-explorer-tab:v1:{...}`, or
//! `--tab-ticket T` from the host's explorer.tabDetach) go through the backend's one-use tickets as before:
//! explorer.tabClaim, import and load, explorer.tabCommit; explorer.tabRelease when anything fails.
//! A picker has no tab drags.
//!
//! # The session
//!
//! A window that closes keeps its tabs (at most 20: location, history, selection, filter, scroll positions) in
//! `apps.explorer.session` of $XDG_CONFIG_HOME/win11-gnome/window-state.json (WIN11_GNOME_CONFIG_DIR overrides
//! the folder), the WebKit host's file, with its lock and its private-file rules, so the tabs survive a switch
//! between the WebKit and the native Explorer. A window opened without arguments gets the last saved tabs back.
//! The window size is the kit's (native-windows.json).
use crate::explorer::{Explorer, Params, WeakExplorer};
use crate::i18n::s;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// The drag type of a tab (tabtransfer.js TAB_MIME).
pub const TAB_MIME: &str = "application/x-win11-explorer-tab+json";
/// A WebKit Explorer's plain-text envelope (tabtransfer.js TAB_TEXT_PREFIX).
const TAB_TEXT_PREFIX: &str = "win11-explorer-tab:v1:";
const KINDS: [&str; 6] = ["home", "gallery", "thispc", "trash", "folder", "search"];
const SESSION_MAX: usize = 1 << 20;
const STATE_FILE_MAX: u64 = 8 << 20;

// ======================================================================================= tab state

fn invalid() -> String {
    s("tabInvalid").to_owned()
}

/// A JavaScript string length (UTF-16 units), as tabtransfer.js counts.
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn location(v: &Value) -> Result<Value, String> {
    let kind = v.get("kind").and_then(Value::as_str).filter(|k| KINDS.contains(k)).ok_or_else(invalid)?;
    let mut out = json!({ "kind": kind });
    if kind == "folder" || kind == "search" {
        let p = v["path"].as_str().filter(|p| p.starts_with('/') && js_len(p) <= 16384).ok_or_else(invalid)?;
        out["path"] = json!(p);
    }
    if kind == "search" {
        let q = v["query"].as_str().filter(|q| js_len(q) <= 4096).ok_or_else(invalid)?;
        out["query"] = json!(q);
    }
    Ok(out)
}

fn path_list(v: &Value) -> Result<Vec<Value>, String> {
    let list = v.as_array().filter(|a| a.len() <= 10000).ok_or_else(invalid)?;
    list.iter()
        .map(|p| p.as_str().filter(|p| p.starts_with('/') && js_len(p) <= 16384).map(|p| json!(p)).ok_or_else(invalid))
        .collect()
}

fn one_path(v: &Value) -> Result<Value, String> {
    path_list(&json!([v])).map(|mut l| l.remove(0))
}

/// JavaScript's Number.isInteger with a lower bound of 0.
fn index(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f < 1e15).map(|f| f as u64))
}

/// JavaScript truthiness (`value.selection || []`).
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Validate a tab state from another window, process or the session (tabtransfer.js normalizeTabState): only
/// the semantic fields, never listings; Err with "tabInvalid". Scroll positions come sorted by key.
pub fn normalize_tab_state(v: &Value) -> Result<Value, String> {
    if !v.is_object() || v["version"].as_f64() != Some(1.0) {
        return Err(invalid());
    }
    let loc = location(&v["loc"])?;
    let history = v["history"].as_array().filter(|h| !h.is_empty() && h.len() <= 60).ok_or_else(invalid)?;
    let history = history.iter().map(location).collect::<Result<Vec<_>, _>>()?;
    let h_index = index(&v["hIndex"]).filter(|i| (*i as usize) < history.len()).ok_or_else(invalid)?;
    let filter = v["filter"].as_str().filter(|f| js_len(f) <= 4096).ok_or_else(invalid)?;
    let memo = v["scrollMemo"].as_array().filter(|m| m.len() <= 120).ok_or_else(invalid)?;
    let mut memo = memo
        .iter()
        .map(|e| {
            let pair = e.as_array().filter(|p| p.len() == 2).ok_or_else(invalid)?;
            let key = pair[0].as_str().filter(|k| js_len(k) <= 20000).ok_or_else(invalid)?;
            let pos = |k: &str| pair[1].get(k).and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0);
            match (pos("top"), pos("left")) {
                (Some(top), Some(left)) => Ok((key.to_owned(), top, left)),
                _ => Err(invalid()),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    memo.sort_by(|a, b| a.0.cmp(&b.0));
    let selection = if truthy(&v["selection"]) { path_list(&v["selection"])? } else { Vec::new() };
    let focus = if v["focus"].is_null() { Value::Null } else { one_path(&v["focus"])? };
    let pending_select = match &v["pendingSelect"] {
        Value::Null => Value::Null,
        Value::Array(_) => Value::Array(path_list(&v["pendingSelect"])?),
        other => Value::Array(path_list(&json!([other]))?),
    };
    let reveal = if v["pendingReveal"].is_null() { Value::Null } else { one_path(&v["pendingReveal"])? };
    Ok(json!({
        "version": 1,
        "loc": loc,
        "history": history,
        "hIndex": h_index,
        "selection": selection,
        "focus": focus,
        "filter": filter,
        "scrollMemo": memo.into_iter().map(|(k, t, l)| json!([k, { "top": t, "left": l }])).collect::<Vec<_>>(),
        "pendingSelect": pending_select,
        "pendingProps": v["pendingProps"] == true,
        "pendingReveal": reveal,
    }))
}

// ===================================================================================== moving tabs

/// The tab drag that is under way (one at a time, tabtransfer.js `session`).
struct Drag {
    ticket: String,
    source: WeakExplorer,
    tab_id: u64,
    /// The exported state (normalized): the destination imports it, the source compares it at the end.
    state: Value,
    /// One of the windows took the drop: reordered, imported, or dropped on the source's own content.
    handled: Cell<bool>,
    /// Dropped on another Explorer window's content: a new window, as outside.
    elsewhere: Cell<bool>,
    cancel: Cell<Option<gdk::DragCancelReason>>,
    /// Enter / leave of the drag over this process's windows (the fallback without the extension).
    over: Cell<i32>,
    /// When it began: a drag GTK never ended does not block the next one for long.
    began: std::time::Instant,
}

thread_local! {
    static DRAG: RefCell<Option<Rc<Drag>>> = const { RefCell::new(None) };
    /// Tabs torn off into a new window, by the local ticket that window was opened with.
    static DETACHED: RefCell<HashMap<String, Rc<Drag>>> = RefCell::new(HashMap::new());
}

fn current() -> Option<Rc<Drag>> {
    DRAG.with(|d| d.borrow().clone())
}

/// A tab drag of this process is under way (one that GTK never ended counts for two minutes at most).
fn dragging() -> bool {
    current().is_some_and(|d| d.began.elapsed() < Duration::from_secs(120))
}

/// This process in envelopes (tabtransfer.js sourceWindow).
fn source_window() -> String {
    format!("native-{}", std::process::id())
}

fn same(a: &Explorer, b: &Explorer) -> bool {
    Rc::ptr_eq(&a.inner, &b.inner)
}

/// tabtransfer.js fail: an info bar "Tabblad kon niet worden verplaatst" with the reason.
fn fail(x: &Explorer, msg: &str) {
    x.show_error(msg, Some(s("tabMoveFailed")));
}

fn release(x: &Explorer, ticket: &str) {
    x.call("explorer.tabRelease", json!({ "ticket": ticket }), |_, _| {});
}

fn tab_position(x: &Explorer, phase: &str, done: impl FnOnce(Result<Value, kit::CallError>) + 'static) {
    x.ctx().gnome_call("explorer.tabPosition", json!({ "pid": std::process::id(), "phase": phase }), done);
}

/// A tab widget was made (drag source for moving the tab).
pub fn attach_tab(x: &Explorer, tab: &gtk::Widget, id: u64) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let w = x.weak();
    source.connect_prepare(move |_, _, _| {
        let x = w.upgrade()?;
        begin(&x, id)
    });
    let tab_ref = tab.downgrade();
    source.connect_drag_begin(move |src, _| {
        if let Some(t) = tab_ref.upgrade() {
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&t))), 40, 14);
        }
    });
    source.connect_drag_cancel(|_, _, reason| {
        if let Some(d) = current() {
            d.cancel.set(Some(reason));
        }
        false
    });
    source.connect_drag_end(|_, _, _| {
        if let Some(d) = current() {
            // GTK may report the cancel just after the end; the destination's drop ran already
            glib::timeout_add_local_once(Duration::from_millis(120), move || decide(d));
        }
    });
    tab.add_controller(source);
}

/// tabtransfer.js start: export the tab (a tab being renamed can't move), note the drag, give the envelope.
fn begin(x: &Explorer, id: u64) -> Option<gdk::ContentProvider> {
    if x.is_picker() || dragging() {
        return None;
    }
    let state = match x.export_tab(id).and_then(|v| normalize_tab_state(&v)) {
        Ok(v) => v,
        Err(e) => {
            fail(x, &e);
            return None;
        }
    };
    let ticket = glib::uuid_string_random().to_string();
    let envelope = json!({ "ticket": ticket, "sourceWindow": source_window() }).to_string();
    let drag = Rc::new(Drag {
        ticket,
        source: x.weak(),
        tab_id: id,
        state,
        handled: Cell::new(false),
        elsewhere: Cell::new(false),
        cancel: Cell::new(None),
        over: Cell::new(0),
        began: std::time::Instant::now(),
    });
    DRAG.with(|d| *d.borrow_mut() = Some(drag));
    // the extension watches for Escape during this drag (its answer comes with "end")
    tab_position(x, "begin", |_| {});
    Some(gdk::ContentProvider::for_bytes(TAB_MIME, &glib::Bytes::from_owned(envelope.into_bytes())))
}

/// tabtransfer.js end: after the drag, tear the tab off into a new window when nobody took it and it was not
/// cancelled.
fn decide(d: Rc<Drag>) {
    DRAG.with(|cur| {
        let mut cur = cur.borrow_mut();
        if cur.as_ref().is_some_and(|c| Rc::ptr_eq(c, &d)) {
            *cur = None;
        }
    });
    if d.handled.get() {
        return;
    }
    if d.elsewhere.get() {
        detach(d);
        return;
    }
    let Some(reason) = d.cancel.get() else { return };
    if reason == gdk::DragCancelReason::UserCancelled {
        return;
    }
    let Some(x) = d.source.upgrade() else { return };
    tab_position(&x, "end", move |r| {
        let tear = match r {
            // Escape seen by the compositor, or still over the window that the extension measures
            Ok(p) => p["cancelled"] != true && p["outside"] != false,
            Err(_) => reason == gdk::DragCancelReason::NoTarget || d.over.get() <= 0,
        };
        if tear {
            detach(d);
        }
    });
}

/// Open a new window for the dragged tab (the host's explorer.tabDetach, in-process).
fn detach(d: Rc<Drag>) {
    if d.source.upgrade().is_none_or(|x| x.tab(d.tab_id).is_none()) {
        return;
    }
    let ticket = d.ticket.clone();
    DETACHED.with(|m| m.borrow_mut().insert(ticket.clone(), d));
    Explorer::new(Params { tab_ticket: Some(ticket), ..Default::default() });
}

/// The destination has the tab: the source closes its own, unless it changed during the move.
fn close_source(d: &Drag) {
    let Some(src) = d.source.upgrade() else { return };
    if src.tab(d.tab_id).is_none() {
        return;
    }
    let unchanged = src.export_tab(d.tab_id).and_then(|v| normalize_tab_state(&v)).is_ok_and(|v| v == d.state);
    if unchanged {
        src.close_tab(d.tab_id);
    } else {
        fail(&src, s("tabKeptChanged"));
    }
}

/// Where a tab dropped at `px` on the strip goes: before the tab under the pointer, or after it when the
/// pointer is on its right half; None: at the end.
fn before_at(x: &Explorer, strip: &gtk::Widget, px: f64) -> Option<u64> {
    let ids = x.tab_ids();
    for (n, id) in ids.iter().enumerate() {
        let Some(w) = x.inner.tab_strip.tab_widget(*id) else { continue };
        let Some(b) = w.compute_bounds(strip) else { continue };
        let (left, width) = (b.x() as f64, b.width() as f64);
        if px >= left && px < left + width {
            return if px >= left + width / 2.0 { ids.get(n + 1).copied() } else { Some(*id) };
        }
    }
    None
}

/// Whether a drag over the strip is (or may be) a tab: ours, or plain text without files (tabtransfer.js
/// isTabDrag; the text is checked on drop).
fn is_tab_drag(drop: &gdk::Drop) -> bool {
    let formats = drop.formats();
    if formats.contain_mime_type(TAB_MIME) {
        return true;
    }
    let mimes: Vec<String> = formats.mime_types().iter().map(|m| m.to_string()).collect();
    let text = mimes.iter().any(|m| m == "text/plain" || m.starts_with("text/plain;"));
    let files = mimes.iter().any(|m| m == "text/uri-list" || m == "application/vnd.portal.filetransfer" || m == "application/vnd.portal.files");
    text && !files
}

/// The tab strip (drop target for tabs from this and other windows); called once per window. The window
/// itself catches tab drops anywhere else, so a drop on an Explorer window is never a tear-off by accident.
pub fn attach_strip(x: &Explorer, strip: &gtk::Widget) {
    let formats = gdk::ContentFormats::new(&[TAB_MIME, "text/plain;charset=utf-8", "text/plain"]);
    let target = gtk::DropTargetAsync::new(Some(formats), gdk::DragAction::MOVE);
    target.connect_accept(|_, drop| is_tab_drag(drop));
    let motion = |_: &gtk::DropTargetAsync, drop: &gdk::Drop, _: f64, _: f64| {
        if is_tab_drag(drop) { gdk::DragAction::MOVE } else { gdk::DragAction::empty() }
    };
    target.connect_drag_enter(motion);
    target.connect_drag_motion(motion);
    let w = x.weak();
    let strip_ref = strip.downgrade();
    target.connect_drop(move |_, drop, px, _| {
        let (Some(x), Some(strip)) = (w.upgrade(), strip_ref.upgrade()) else { return false };
        strip_drop(&x, &strip, drop, px)
    });
    strip.add_controller(target);

    let window = x.window().window();
    let catcher = gtk::DropTargetAsync::new(Some(gdk::ContentFormats::new(&[TAB_MIME])), gdk::DragAction::MOVE);
    catcher.connect_accept(|_, drop| drop.formats().contain_mime_type(TAB_MIME) && current().is_some());
    let any = |_: &gtk::DropTargetAsync, _: &gdk::Drop, _: f64, _: f64| gdk::DragAction::MOVE;
    catcher.connect_drag_enter(any);
    catcher.connect_drag_motion(any);
    let w = x.weak();
    catcher.connect_drop(move |_, drop, _, _| {
        if let (Some(d), Some(x)) = (current(), w.upgrade()) {
            match d.source.upgrade() {
                // a drop within the source window that is not on the tabs is cancelled, never a tear-off
                Some(src) if same(&src, &x) => d.handled.set(true),
                _ => d.elsewhere.set(true),
            }
        }
        drop.finish(gdk::DragAction::empty());
        true
    });
    window.add_controller(catcher);

    let over = gtk::DropControllerMotion::new();
    over.connect_enter(|_, _, _| {
        if let Some(d) = current() {
            d.over.set(d.over.get() + 1);
        }
    });
    over.connect_leave(|_| {
        if let Some(d) = current() {
            d.over.set(d.over.get() - 1);
        }
    });
    window.add_controller(over);
}

/// A drop on the strip: a tab of this process moves here; another process's envelope is claimed.
fn strip_drop(x: &Explorer, strip: &gtk::Widget, drop: &gdk::Drop, px: f64) -> bool {
    let before = before_at(x, strip, px);
    // only this process makes the tab type: the drag under way
    if drop.formats().contain_mime_type(TAB_MIME)
        && let Some(d) = current()
    {
        d.handled.set(true);
        drop.finish(gdk::DragAction::MOVE);
        move_here(x, d, before);
        return true;
    }
    let w = x.weak();
    let drop = drop.clone();
    glib::spawn_future_local(async move {
        let envelope = read_envelope(&drop).await;
        drop.finish(if envelope.is_some() { gdk::DragAction::MOVE } else { gdk::DragAction::empty() });
        let (Some((ticket, source)), Some(x)) = (envelope, w.upgrade()) else { return };
        if source == source_window() {
            match current().filter(|d| d.ticket == ticket) {
                Some(d) => {
                    d.handled.set(true);
                    move_here(&x, d, before);
                }
                None => fail(&x, &invalid()),
            }
            return;
        }
        accept_remote(&x, ticket, source, before);
    });
    true
}

/// A tab of this process dropped on `x`'s strip: reorder in its own window, else import it here.
fn move_here(x: &Explorer, d: Rc<Drag>, before: Option<u64>) {
    let Some(src) = d.source.upgrade() else { return };
    if same(&src, x) {
        x.move_tab_before(d.tab_id, before);
        return;
    }
    x.import_tab(&d.state.clone(), before, move |x, r| match r {
        Ok(_) => close_source(&d),
        Err(e) => fail(x, &e),
    });
}

/// The envelope of another process's tab drag (tabtransfer.js payload): (ticket, sourceWindow).
async fn read_envelope(drop: &gdk::Drop) -> Option<(String, String)> {
    let ours = drop.formats().contain_mime_type(TAB_MIME);
    let mimes: &[&str] = if ours { &[TAB_MIME] } else { &["text/plain;charset=utf-8", "text/plain"] };
    let (stream, _) = drop.read_future(mimes, glib::Priority::DEFAULT).await.ok()?;
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let chunk = stream.read_bytes_future(16384, glib::Priority::DEFAULT).await.ok()?;
        if chunk.is_empty() {
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 65536 {
            return None;
        }
    }
    let _ = stream.close_future(glib::Priority::DEFAULT).await;
    let text = String::from_utf8(bytes).ok()?;
    let encoded = if ours { text.as_str() } else { text.strip_prefix(TAB_TEXT_PREFIX)? };
    let v: Value = serde_json::from_str(encoded).ok()?;
    let ticket = v["ticket"].as_str().filter(|t| (32..=36).contains(&t.len()) && t.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'))?;
    let source = v["sourceWindow"].as_str().filter(|s| !s.is_empty())?;
    Some((ticket.to_owned(), source.to_owned()))
}

/// tabtransfer.js accept for another process's tab: claim, check the source, import and load, commit.
fn accept_remote(x: &Explorer, ticket: String, source: String, before: Option<u64>) {
    x.call("explorer.tabClaim", json!({ "ticket": ticket }), move |x, r| {
        let prev = x.active_id();
        let state = r.map_err(|e| e.message).and_then(|claim| {
            if claim["sourceWindow"].as_str() != Some(source.as_str()) {
                return Err(invalid());
            }
            normalize_tab_state(&claim["state"])
        });
        let state = match state {
            Ok(s) => s,
            Err(e) => {
                release(x, &ticket);
                fail(x, &e);
                return;
            }
        };
        x.import_tab(&state, before, move |x, r| match r {
            Err(e) => {
                release(x, &ticket);
                fail(x, &e);
            }
            Ok(id) => {
                let t = ticket.clone();
                x.call("explorer.tabCommit", json!({ "ticket": ticket }), move |x, r| {
                    if let Err(e) = r {
                        x.rollback_imported_tab(id, prev);
                        release(x, &t);
                        fail(x, &e.message);
                    }
                });
            }
        });
    });
}

/// The window was opened with `--tab-ticket T`: claim the tab, `x.import_tab` it, commit (or release).
/// The window starts with a placeholder Start tab that the imported tab replaces. A ticket of a tab torn off
/// in this process needs no backend.
pub fn claim_ticket(x: &Explorer, ticket: &str) {
    if let Some(d) = DETACHED.with(|m| m.borrow_mut().remove(ticket)) {
        x.import_tab(&d.state.clone(), None, move |x, r| match r {
            Ok(_) => close_source(&d),
            Err(e) => x.show_error(&e, None),
        });
        return;
    }
    // main.js start: claim, load, commit; on failure release and say why
    let ticket = ticket.to_owned();
    x.call("explorer.tabClaim", json!({ "ticket": ticket }), move |x, r| {
        let prev = x.active_id();
        let state = r.map_err(|e| e.message).and_then(|claim| normalize_tab_state(&claim["state"]));
        let state = match state {
            Ok(s) => s,
            Err(e) => {
                release(x, &ticket);
                x.show_error(&e, None);
                return;
            }
        };
        x.import_tab(&state, None, move |x, r| match r {
            Err(e) => {
                release(x, &ticket);
                x.show_error(&e, None);
            }
            Ok(id) => {
                let t = ticket.clone();
                x.call("explorer.tabCommit", json!({ "ticket": ticket }), move |x, r| {
                    if let Err(e) = r {
                        release(x, &t);
                        x.rollback_imported_tab(id, prev);
                        x.show_error(&e.message, None);
                    }
                });
            }
        });
    });
}

// ========================================================================================= session

/// The host's config folder (runtime.rs: WIN11_GNOME_CONFIG_DIR, else XDG_CONFIG_HOME/win11-gnome).
fn config_dir() -> PathBuf {
    std::env::var_os("WIN11_GNOME_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(glib::user_config_dir)
            .join("win11-gnome")
    })
}

fn private_error(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, what.to_owned())
}

/// window_state.rs private_file: ours, one link, no group or other bits, no symlink.
fn private_file(path: &Path, create: bool) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let st = f.metadata()?;
    if !st.is_file() || st.uid() != unsafe { libc::geteuid() } || st.nlink() != 1 || st.mode() & 0o077 != 0 {
        return Err(private_error("Window state must be private"));
    }
    Ok(f)
}

/// The window-state lock (window_state.rs locked); released when the file closes.
fn locked(config: &Path) -> io::Result<File> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(config)?;
    let st = fs::symlink_metadata(config)?;
    if !st.is_dir() || st.file_type().is_symlink() || st.uid() != unsafe { libc::geteuid() } {
        return Err(private_error("Invalid window-state directory"));
    }
    let f = private_file(&config.join(".window-state.lock"), true)?;
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(f);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// window-state.json ({version: 1, apps: {...}}; a missing or broken file is an empty one).
fn read_root(config: &Path) -> io::Result<Value> {
    let f = match private_file(&config.join("window-state.json"), false) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(json!({ "version": 1, "apps": {} })),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    f.take(STATE_FILE_MAX + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > STATE_FILE_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Window-state file is too large"));
    }
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(if v["version"] == 1 && v["apps"].is_object() { v } else { json!({ "version": 1, "apps": {} }) })
}

/// window_state.rs session: {version: 1, tabs: [1..=20 valid tab states], active}, at most 1 MB.
fn session(v: &Value) -> Option<Value> {
    let tabs = v["tabs"].as_array().filter(|t| !t.is_empty() && t.len() <= 20)?;
    if v["version"].as_f64() != Some(1.0) || serde_json::to_vec(v).ok()?.len() > SESSION_MAX {
        return None;
    }
    let tabs = tabs.iter().map(|t| normalize_tab_state(t).ok()).collect::<Option<Vec<_>>>()?;
    let active = index(&v["active"]).filter(|a| (*a as usize) < tabs.len())?;
    Some(json!({ "version": 1, "tabs": tabs, "active": active }))
}

/// The tabs to restore in a window opened without arguments ({version: 1, tabs: [state], active}), if any.
pub fn restored_session() -> Option<Value> {
    let config = config_dir();
    if !config.join("window-state.json").exists() {
        return None;
    }
    let root = {
        let _lock = locked(&config).ok()?;
        read_root(&config).ok()?
    };
    session(&root["apps"]["explorer"]["session"])
}

/// A tab state that fits the session rules: scroll positions and history trimmed, else its location only.
fn session_tab(t: &Value) -> Value {
    let mut t = t.clone();
    if let Some(memo) = t["scrollMemo"].as_array_mut() {
        memo.truncate(120);
    }
    if let Ok(v) = normalize_tab_state(&t) {
        return v;
    }
    let loc = location(&t["loc"]).unwrap_or_else(|_| json!({ "kind": "home" }));
    json!({ "version": 1, "loc": loc, "history": [loc], "hIndex": 0, "selection": [], "focus": null, "filter": "",
        "scrollMemo": [], "pendingSelect": null, "pendingProps": false, "pendingReveal": null })
}

/// A window's tabs as a session ({version: 1, tabs, active}, main.js saveWindowSession's rules), or None (no tabs,
/// too big even with the locations only).
fn window_session(x: &Explorer) -> Option<Value> {
    let state = x.session_state();
    let mut tabs: Vec<Value> = state["tabs"].as_array().into_iter().flatten().map(session_tab).collect();
    if tabs.is_empty() {
        return None;
    }
    let active = index(&state["active"]).map(|a| a as usize).unwrap_or(0).min(tabs.len() - 1);
    let size = |tabs: &Vec<Value>| serde_json::to_vec(&json!({ "version": 1, "tabs": tabs, "active": active })).map(|b| b.len()).unwrap_or(usize::MAX);
    if size(&tabs) > SESSION_MAX {
        // main.js saveWindowSession: only the locations when it gets too big
        for t in tabs.iter_mut() {
            let loc = t["loc"].clone();
            *t = json!({ "version": 1, "loc": loc, "history": [loc], "hIndex": 0, "selection": [], "focus": null,
                "filter": t["filter"].clone(), "scrollMemo": [], "pendingSelect": null, "pendingProps": false,
                "pendingReveal": null });
        }
    }
    if size(&tabs) > SESSION_MAX {
        return None;
    }
    Some(json!({ "version": 1, "tabs": tabs, "active": active }))
}

/// The window closes: keep its tabs (`x.session_state()`) for the next window without arguments.
pub fn save_session(x: &Explorer) {
    let Some(value) = window_session(x) else { return };
    // the last window's write must finish before the process ends
    let hold = x.ctx().app().hold();
    kit::spawn(
        move || write_session(&config_dir(), &value),
        move |r: io::Result<()>| {
            if let Err(e) = r {
                eprintln!("win11-gtk-explorer: saving the tabs failed: {e}");
            }
            drop(hold);
        },
    );
}

/// Merge the session into window-state.json under its lock (the WebKit host's normalRect / maximized stay).
fn write_session(config: &Path, session: &Value) -> io::Result<()> {
    let _lock = locked(config)?;
    let mut root = read_root(config)?;
    if !root["apps"]["explorer"].is_object() {
        root["apps"]["explorer"] = json!({});
    }
    root["apps"]["explorer"]["session"] = session.clone();
    let bytes = serde_json::to_vec(&root)?;
    if bytes.len() as u64 > STATE_FILE_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Window-state file is too large"));
    }
    let temp = config.join(format!(".window-state.{}.native.tmp", std::process::id()));
    let _ = fs::remove_file(&temp);
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        f.write_all(&bytes)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.sync_all()?;
        fs::rename(&temp, config.join("window-state.json"))?;
        if let Ok(dir) = File::open(config) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({"version":1,"loc":{"kind":"folder","path":"/a"},"history":[{"kind":"home"},{"kind":"folder","path":"/a"}],
            "hIndex":1,"selection":["/a/x"],"focus":"/a/x","filter":"","scrollMemo":[["folder:/a",{"top":10,"left":0}]],
            "pendingSelect":null,"pendingProps":false,"pendingReveal":null,"items":[{"name":"dropped"}]})
    }

    #[test]
    fn tab_states_validate_and_drop_listings() {
        let v = normalize_tab_state(&sample()).unwrap();
        assert!(v.get("items").is_none());
        assert_eq!(v["hIndex"], 1);
        for mutate in 0..7 {
            let mut v = sample();
            match mutate {
                0 => v["version"] = json!(2),
                1 => v["loc"]["path"] = json!("relative"),
                2 => v["history"] = json!([]),
                3 => v["hIndex"] = json!(2),
                4 => v["selection"] = json!(["bad"]),
                5 => v["scrollMemo"][0][1]["top"] = json!(-1),
                _ => v["loc"] = json!({"kind":"elsewhere"}),
            }
            assert!(normalize_tab_state(&v).is_err(), "mutation {mutate}");
        }
        let mut v = sample();
        v["pendingSelect"] = json!("/a/y");
        assert_eq!(normalize_tab_state(&v).unwrap()["pendingSelect"], json!(["/a/y"]));
    }

    #[test]
    fn sessions_round_trip_through_the_host_file() {
        let dir = std::env::temp_dir().join(format!("win11-explorer-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let other = json!({"version":1,"apps":{"explorer":{"normalRect":{"x":0,"y":0,"w":800,"h":600},"maximized":false},"settings":{"maximized":true}}});
        let path = dir.join("window-state.json");
        fs::write(&path, serde_json::to_vec(&other).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let s = json!({"version":1,"tabs":[normalize_tab_state(&sample()).unwrap()],"active":0});
        write_session(&dir, &s).unwrap();
        let root = read_root(&dir).unwrap();
        assert_eq!(root["apps"]["settings"]["maximized"], true);
        assert_eq!(root["apps"]["explorer"]["normalRect"]["w"], 800);
        assert_eq!(session(&root["apps"]["explorer"]["session"]), Some(s));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }
}

// ========================================================================================= restart

/// At most this many windows come back after a restart.
const RESTART_WINDOWS: usize = 20;

/// The file that carries every window's tabs over a restart (`shell.restartAll`, kit `on_restart`):
/// `$XDG_RUNTIME_DIR/win11-gnome/explorer-restart-<pid>.json`, private; the new image reads and removes it
/// (`--restart-sessions <pid>`, main.rs). The pid names the file, so the argument cannot point anywhere else.
fn restart_file(pid: u32) -> PathBuf {
    glib::user_runtime_dir().join("win11-gnome").join(format!("explorer-restart-{pid}.json"))
}

/// Before a restart (main.rs): the tabs of every window (`windows`, the active one last so it ends in front) into
/// the restart file, and the active window's into window-state.json as closing it would. Synchronous: the exec
/// follows at once. The pid for `--restart-sessions` when the file was written.
pub fn save_for_restart(windows: &[Explorer]) -> Option<u32> {
    let sessions: Vec<Value> = windows.iter().filter_map(window_session).take(RESTART_WINDOWS).collect();
    if let Some(last) = sessions.last()
        && let Err(e) = write_session(&config_dir(), last)
    {
        eprintln!("win11-gtk-explorer: saving the tabs failed: {e}");
    }
    if sessions.is_empty() {
        return None;
    }
    let pid = std::process::id();
    let path = restart_file(pid);
    let bytes = serde_json::to_vec(&json!({ "version": 1, "windows": sessions })).ok()?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temp);
    let result = (|| {
        let dir = path.parent().ok_or_else(|| private_error("Invalid runtime directory"))?;
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(&temp, &path)
    })();
    match result {
        Ok(()) => Some(pid),
        Err(e) => {
            let _ = fs::remove_file(&temp);
            eprintln!("win11-gtk-explorer: keeping the windows over the restart failed: {e}");
            None
        }
    }
}

/// The sessions a restarting image left (`--restart-sessions <pid>`): each checked like a saved session, at most
/// RESTART_WINDOWS; the file goes. Empty when there is none (the saved session of window-state.json applies).
pub fn take_restart_sessions(pid: &str) -> Vec<Value> {
    let Some(pid) = pid.parse::<u32>().ok().filter(|p| *p > 0) else { return Vec::new() };
    let path = restart_file(pid);
    let read = private_file(&path, false).and_then(|f| {
        let mut bytes = Vec::new();
        f.take(STATE_FILE_MAX + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let _ = fs::remove_file(&path);
    let Ok(bytes) = read else { return Vec::new() };
    if bytes.len() as u64 > STATE_FILE_MAX {
        return Vec::new();
    }
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if v["version"] != 1 {
        return Vec::new();
    }
    v["windows"].as_array().into_iter().flatten().take(RESTART_WINDOWS).filter_map(session).collect()
}
