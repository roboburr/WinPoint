//! File operations (6ebf169f:vendor/ui/js/apps/explorer/main.js `op`, `act`, `trashPaths`, `trashNamesFor`,
//! `restoreNames`, `dropTo`, `renameItem`, `setAttributes`, `loadOpenWith`, `showJob`): open, rename, new
//! folder and text document, cut, copy, paste, move and copy by drag and drop, to the Prullenbak (Del and
//! Shift+Del: the WebKit app never deleted permanently, and the host has no such method), compress, extract,
//! restore, empty the Prullenbak, Openen met, Ongedaan maken (Ctrl+Z) and separate transfer windows.
//!
//! Long jobs (copy, move, paste, compress, extract) carry a `jobId`; when one runs longer than 700 ms a
//! Windows-style operation window shows progress, details and host-backed transfer controls. Name conflicts
//! remain the host's ("naam (2).ext"),
//! failed items come back in `failed` and show as "Dit ging mis bij: ...".
//!
//! Undo steps are asynchronous (`Step`): Ctrl+Z runs the last one, then "Ongedaan gemaakt" (or the error), the
//! Prullenbak state and a reload; the toasts' Ongedaan maken runs the same step without those.
use crate::explorer::{Explorer, NavOpts, ToastOpts, WeakExplorer};
use crate::fileview::RenameDone;
use crate::i18n::{f, n, s};
use crate::model::Loc;
use crate::transfer_window::TransferWindow;
use crate::util::{self, Item, num, st};
use gtk::glib;
use gtk::prelude::*;
use kit::CallError;
use kit::controls::{Dialog, DialogResult, MenuItem, Severity};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

// ============================================================================================ helpers

/// A one-shot timer that can be restarted and cancelled (never removes a source that already ran).
#[derive(Clone, Default)]
pub(crate) struct Timer(Rc<RefCell<Option<glib::SourceId>>>);

impl Timer {
    /// Run `f` after `ms` (a pending run is cancelled first).
    pub fn start(&self, ms: u64, f: impl FnOnce() + 'static) {
        self.cancel();
        let cell = self.0.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            // the source is done: forget its id without removing it
            drop(cell.borrow_mut().take());
            f();
        });
        *self.0.borrow_mut() = Some(id);
    }

    pub fn cancel(&self) {
        let id = self.0.borrow_mut().take();
        if let Some(id) = id {
            id.remove();
        }
    }
}

/// The message of a failed call ("Dat lukte niet" when it has none).
pub(crate) fn err_text(e: &CallError) -> String {
    let m = e.to_string();
    if m.trim().is_empty() { s("errTitle").to_owned() } else { m }
}

fn show_err(x: &Explorer, e: &CallError) {
    x.show_error(&e.to_string(), None);
}

/// The paths of items (drives left out).
fn paths_of(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.get("volume").is_none_or(|v| !v.is_object()))
        .map(|i| st(i, "path").to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn failed(res: &Value) -> &[Value] {
    res["failed"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn failed_path(v: &Value) -> String {
    match v {
        Value::String(p) => p.clone(),
        _ => [st(v, "path"), st(v, "name")].into_iter().find(|p| !p.is_empty()).unwrap_or("").to_owned(),
    }
}

/// main.js failedNames: the first five names, " …" for more.
fn failed_names(res: &Value) -> String {
    let list = failed(res);
    let mut out = list.iter().take(5).map(|v| util::basename(&failed_path(v))).collect::<Vec<_>>().join(", ");
    if list.len() > 5 {
        out.push_str(" \u{2026}");
    }
    out
}

/// "Dit ging mis bij: ..." when some items failed.
fn report_failed(x: &Explorer, res: &Value) {
    if !failed(res).is_empty() {
        x.show_error(&failure_message(res), None);
    }
}

fn failure_message(res: &Value) -> String {
    let mut message = f("errSome", &[("names", &failed_names(res))]);
    // A cancelled transfer may deliberately preserve a partial destination changed by another program.
    // Keep the host's concrete warning visible instead of misreporting a clean cancellation.
    if let Some(warning) = failed(res).iter().filter(|entry| entry["code"] == "partial_cleanup")
        .find_map(|entry| entry["error"].as_str().filter(|message| !message.is_empty()))
    {
        message.push('\n');
        message.push_str(warning);
    }
    message
}

fn dry(res: &Value) -> bool {
    res["dryRun"] == true
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().filter(|p| !p.is_empty()).map(str::to_owned)
}

// ============================================================================================ jobs

/// The methods that get a jobId and a separate operation window, with their job kind.
const JOB_OPS: [(&str, &str); 5] =
    [("files.copy", "copy"), ("files.move", "move"), ("files.paste", "paste"), ("files.compress", "compress"), ("files.extract", "extract")];

struct Job {
    id: String,
    kind: &'static str,
    count: usize,
    dest: Option<String>,
    x: WeakExplorer,
    source_label: String,
    source_dir: Option<String>,
    destination_label: String,
    window: RefCell<Option<TransferWindow>>,
    /// A hidden/minimised operation and a closed source Explorer must not kill its backend mid-transfer.
    hold: RefCell<Option<gtk::gio::ApplicationHoldGuard>>,
    backend_epoch: u64,
    last: RefCell<Option<Value>>,
    conflict: RefCell<Option<Value>>,
    /// The call gave up waiting (the backend client's read limit) while files.progress still came: finish when
    /// it says the job finished.
    orphan: Cell<bool>,
}

thread_local! {
    static JOBS: RefCell<HashMap<String, Rc<Job>>> = RefCell::new(HashMap::new());
    static JOB_SEQ: Cell<u64> = const { Cell::new(0) };
    static LISTENING: Cell<bool> = const { Cell::new(false) };
    static BACKEND_EPOCH: Cell<u64> = const { Cell::new(0) };
}

fn same_operation<T>(current: Option<Rc<T>>, expected: &Rc<T>) -> bool {
    current.is_some_and(|current| Rc::ptr_eq(&current, expected))
}

fn job_kind(method: &str) -> Option<&'static str> {
    JOB_OPS.iter().find(|(m, _)| *m == method).map(|(_, k)| *k)
}

fn job(id: &str) -> Option<Rc<Job>> {
    JOBS.with(|j| j.borrow().get(id).cloned())
}

/// One listener for the whole process: files.progress {jobId, done, total, current, finished}.
fn listen_progress(x: &Explorer) {
    if LISTENING.with(|l| l.replace(true)) {
        return;
    }
    x.ctx().on("backend.ready", |_, data| {
        if data["respawn"] != true { return; }
        BACKEND_EPOCH.with(|epoch| epoch.set(epoch.get().wrapping_add(1)));
        // Do not discard requests waiting to start on the new endpoint. Only disconnected old calls are
        // known orphans; old in-flight callbacks below detect the epoch change when they return offline.
        let orphaned: Vec<Rc<Job>> = JOBS.with(|jobs| jobs.borrow().values().filter(|job| job.orphan.get()).cloned().collect());
        for job in orphaned {
            JOBS.with(|jobs| jobs.borrow_mut().remove(&job.id));
            remove_window(&job, Some(s("transferInterrupted")));
        }
    });
    x.ctx().on("files.conflict", |_, data| {
        let Some(id) = data["jobId"].as_str() else { return };
        let Some(job) = self::job(id) else { return };
        if data["conflictId"].as_str().is_none_or(str::is_empty) { return; }
        *job.conflict.borrow_mut() = Some(data.clone());
        if job.window.borrow().is_none() { show_window(&job); }
        show_conflict(&job, data);
    });
    x.ctx().on("files.progress", |_, d| {
        let Some(id) = d["jobId"].as_str() else { return };
        let Some(job) = job(id) else { return };
        *job.last.borrow_mut() = Some(d.clone());
        if d["state"].as_str().is_some_and(|state| state != "waitingConflict") {
            job.conflict.borrow_mut().take();
        }
        if let Some(window) = job.window.borrow().as_ref() {
            window.update(d);
        }
        if job.orphan.get() && d["finished"] == true {
            JOBS.with(|j| j.borrow_mut().remove(id));
            remove_window(&job, (d["state"] == "failed").then_some(s("transferFailed")));
            if let Some(x) = job.x.upgrade() {
                if let Some(dest) = &job.dest {
                    x.invalidate_dir(dest);
                }
                x.load_clipboard();
                x.load_trash_info();
                x.reload_active(0);
            }
        }
    });
}

fn show_window(job: &Job) {
    if job.window.borrow().is_some() { return; }
    let id = job.id.clone();
    let window = TransferWindow::new(job.kind, job.count, job.source_label.clone(), job.destination_label.clone(),
        job.source_dir.clone(), job.dest.clone(), Rc::new(move |action| control_job(&id, action)));
    if let Some(last) = job.last.borrow().as_ref() {
        window.update(last);
    }
    let activate = job.x.upgrade().is_some_and(|x| x.window().window().is_active());
    window.show(activate);
    *job.window.borrow_mut() = Some(window);
    if let Some(conflict) = job.conflict.borrow().as_ref() { show_conflict(job, conflict); }
}

fn show_conflict(job: &Job, data: &Value) {
    let (id, conflict_id) = (job.id.clone(), data["conflictId"].as_str().unwrap_or("").to_owned());
    if let Some(window) = job.window.borrow().as_ref() {
        window.show_conflict(data, Rc::new(move |choice, all| resolve_conflict(&id, &conflict_id, choice, all)));
    }
}

fn resolve_conflict(id: &str, conflict_id: &str, choice: &str, all: bool) {
    let Some(expected) = self::job(id) else { return };
    if expected.conflict.borrow().as_ref().is_none_or(|data| data["conflictId"] != conflict_id) { return; }
    let (id, conflict_id) = (id.to_owned(), conflict_id.to_owned());
    kit::ctx().call("files.resolveConflict", json!({"jobId": id, "conflictId": conflict_id,
        "choice": choice, "applyToAll": all}), move |result| {
        if !same_operation(self::job(&id), &expected) { return; }
        if let Err(error) = result
            && let Some(window) = expected.window.borrow().as_ref()
        {
            window.conflict_error(&conflict_id, &err_text(&error));
        }
        // Success is only a recorded decision. The worker's next progress/conflict event controls the page.
    });
}

fn remove_window(job: &Job, error: Option<&str>) {
    if error.is_some() && job.window.borrow().is_none() { show_window(job); }
    let window = job.window.borrow_mut().take();
    if let Some(window) = window {
        if let Some(error) = error { window.fail(error); } else { window.finish(); }
    }
    let hold = job.hold.borrow_mut().take();
    drop(hold);
}

fn control_job(id: &str, action: &str) {
    let Some(job) = self::job(id) else { return };
    let Some(stamp) = job.window.borrow().as_ref().map(TransferWindow::begin_control) else { return };
    let id = job.id.clone();
    kit::ctx().call("files.transferControl", json!({"jobId": id, "action": action}), move |result| {
        let Some(job) = self::job(&id) else { return };
        if let Some(window) = job.window.borrow().as_ref() {
            match result {
                Ok(value) => window.control_result(&value, stamp),
                Err(error) => window.control_error(&err_text(&error), stamp),
            }
        }
    });
}

/// main.js `op`: a files.* call; copy, move, paste, compress and extract get a jobId and, after 700 ms, a
/// progress window. A dry-run answer says so once.
pub(crate) fn op(x: &Explorer, method: &str, mut args: Value, done: impl FnOnce(&Explorer, Result<Value, CallError>) + 'static) {
    let Some(kind) = job_kind(method) else {
        x.call(method, args, move |x, r| {
            if let Ok(v) = &r {
                x.check_dry(v);
            }
            done(x, r);
        });
        return;
    };
    listen_progress(x);
    let seq = JOB_SEQ.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let id = format!("xp-{:x}-{seq}", kit::format::now_ms().max(0.0) as u64);
    let mut count = match args["paths"].as_array() {
        Some(a) => a.iter().filter(|p| p.as_str().is_some_and(|p| !p.is_empty())).count(),
        None => usize::from(args["path"].as_str().is_some_and(|p| !p.is_empty())),
    };
    if count == 0 {
        count = x.state().clipboard_paths.len().max(1);
    }
    let sources: Vec<String> = if let Some(paths) = args["paths"].as_array() {
        paths.iter().filter_map(Value::as_str).map(str::to_owned).collect()
    } else if let Some(path) = args["path"].as_str() {
        vec![path.to_owned()]
    } else { x.state().clipboard_paths.clone() };
    let parents: HashSet<String> = sources.iter().map(|path| util::dirname(path)).collect();
    let source_label = if parents.len() == 1 {
        parents.iter().next().map(|path| x.folder_label(path)).unwrap_or_default()
    } else { s("transferVarious").to_owned() };
    let source_dir = (parents.len() == 1).then(|| parents.iter().next().cloned()).flatten();
    let dest = text_of(&args, "dest");
    let destination_label = dest.as_deref().map(|path| x.folder_label(path)).unwrap_or_default();
    let kind = if kind == "paste" {
        if x.state().clipboard_op.as_deref() == Some("cut") { "move" } else { "copy" }
    } else { kind };
    let job = Rc::new(Job {
        id: id.clone(),
        kind,
        count,
        dest,
        x: x.weak(),
        source_label,
        source_dir,
        destination_label,
        window: RefCell::new(None),
        hold: RefCell::new(Some(x.ctx().app().hold())),
        backend_epoch: BACKEND_EPOCH.with(Cell::get),
        last: RefCell::new(None),
        conflict: RefCell::new(None),
        orphan: Cell::new(false),
    });
    JOBS.with(|j| j.borrow_mut().insert(id.clone(), job.clone()));
    let timer = Timer::default();
    let tid = id.clone();
    timer.start(700, move || {
        if let Some(job) = self::job(&tid) {
            show_window(&job);
        }
    });
    args["jobId"] = json!(id);
    if matches!(method, "files.copy" | "files.move" | "files.paste") { args["interactive"] = json!(true); }
    // Job ownership is process-wide, unlike x.call's source-window-bound callback. Always finish the window
    // and release its hold even if the user closed the initiating Explorer while this request was running.
    x.ctx().call(method, args, move |r| {
        timer.cancel();
        // A respawn cleanup may already have retired this exact operation. Its late response cannot reopen
        // an error window, release another job, or invoke the source-window completion a second time.
        if !same_operation(self::job(&id), &job) { return; }
        if let Err(e) = &r
            && e.code == "offline"
            && job.last.borrow().as_ref().is_some_and(|l| l["finished"] != true)
        {
            if BACKEND_EPOCH.with(Cell::get) != job.backend_epoch {
                JOBS.with(|jobs| jobs.borrow_mut().remove(&id));
                remove_window(&job, Some(s("transferInterrupted")));
            } else {
                // Still potentially running in the live host. Preserve ownership until finish or respawn.
                job.orphan.set(true);
            }
            return;
        }
        JOBS.with(|j| j.borrow_mut().remove(&id));
        let error = match &r {
            Err(error) => Some(err_text(error)),
            Ok(value) if !failed(value).is_empty() => Some(failure_message(value)),
            _ => None,
        };
        remove_window(&job, error.as_deref());
        if let Some(x) = job.x.upgrade() {
            if let Ok(v) = &r { x.check_dry(v); }
            done(&x, r);
        }
    });
}

// ============================================================================================ undo

/// What an undo step reports when it is done (Err: the message).
pub(crate) type StepDone = Box<dyn FnOnce(&Explorer, Result<(), String>)>;
/// One undo step: run it, then `done`.
type Step = Rc<dyn Fn(&Explorer, StepDone)>;

fn noop() -> StepDone {
    Box::new(|_, _| {})
}

/// Push an undo step; Ctrl+Z runs it, then "Ongedaan gemaakt" or the error, the Prullenbak and a reload.
fn push_step(x: &Explorer, label: &str, step: Step) {
    x.push_undo(label, move |x| {
        step(
            x,
            Box::new(|x, r| {
                match r {
                    Ok(()) => {
                        x.toast(s("undone"), ToastOpts { timeout: 2000, ..Default::default() });
                    }
                    Err(m) => x.show_error(&m, None),
                }
                x.load_trash_info();
                x.reload_active(100);
            }),
        );
    });
}

/// A toast action that runs an undo step (without the Ctrl+Z toast).
fn undo_action(x: &Explorer, step: Step) -> Option<(String, Rc<dyn Fn()>)> {
    let w = x.weak();
    Some((
        s("undo").to_owned(),
        Rc::new(move || {
            if let Some(x) = w.upgrade() {
                step(&x, noop());
            }
        }),
    ))
}

fn trash_step(path: String) -> Step {
    Rc::new(move |x, done| {
        op(x, "files.trash", json!({ "paths": [path.clone()] }), move |x, r| done(x, r.map(|_| ()).map_err(|e| err_text(&e))));
    })
}

/// Ongedaan maken (Ctrl+Z): the last step.
pub fn undo(x: &Explorer) {
    if let Some(u) = x.pop_undo() {
        (u.run)(x);
    }
}

// ============================================================================================ open, rename

/// Open a file with its default app (files.open; untrusted launchers ask first, the kit does that).
pub fn open_file(x: &Explorer, item: &Item) {
    op(x, "files.open", json!({ "path": st(item, "path") }), |x, r| {
        if let Err(e) = r {
            show_err(x, &e);
        }
    });
}

/// main.js renameItem: files.rename, the undo step, "nameHidden" for a new leading dot while hidden items are
/// hidden, and the renamed item selected after the reload; `done(Ok(new path))` or `done(Err(message))`.
pub(crate) fn rename_path(
    x: &Explorer,
    path: String,
    old_name: String,
    new_name: String,
    done: impl FnOnce(&Explorer, Result<String, String>) + 'static,
) {
    op(x, "files.rename", json!({ "path": path, "newName": new_name }), move |x, r| match r {
        Ok(v) => {
            let np = text_of(&v, "path").unwrap_or_else(|| util::join_path(&util::dirname(&path), &new_name));
            let (back, old) = (np.clone(), old_name.clone());
            push_step(
                x,
                "rename",
                Rc::new(move |x, done| {
                    op(x, "files.rename", json!({ "path": back.clone(), "newName": old.clone() }), move |x, r| {
                        done(x, r.map(|_| ()).map_err(|e| err_text(&e)));
                    });
                }),
            );
            if new_name.starts_with('.') && !old_name.starts_with('.') && !x.cfg_bool("showHidden", false) {
                x.toast(s("nameHidden"), ToastOpts { severity: Some(Severity::Info), ..Default::default() });
            }
            x.select_after_reload(std::slice::from_ref(&np), None);
            done(x, Ok(np));
        }
        Err(e) => done(x, Err(err_text(&e))),
    });
}

/// The inline rename's commit: `done(Ok)` closes the box, `done(Err(text))` keeps it open with the message.
pub fn rename_item(x: &Explorer, item: &Item, new_name: &str, done: RenameDone) {
    let path = st(item, "path").to_owned();
    let old = match st(item, "name") {
        "" => util::basename(&path),
        n => n.to_owned(),
    };
    rename_path(x, path, old, new_name.to_owned(), move |_, r| done(r.map(|_| ())));
}

/// What files.setAttributes did (main.js setAttributes).
pub(crate) enum Attr {
    Done,
    /// The host has no attributes: hide by a leading dot instead.
    RenameHidden,
}

/// Alleen-lezen / Verborgen of the Eigenschappen dialog (files.setAttributes {path, readOnly?, hidden?}).
pub(crate) fn set_attributes(x: &Explorer, args: Value, hidden_fallback: bool, done: impl FnOnce(&Explorer, Result<Attr, String>) + 'static) {
    let has_hidden = !args["hidden"].is_null();
    x.call("files.setAttributes", args, move |x, r| match r {
        Ok(v) => {
            x.check_dry(&v);
            x.reload_active(150);
            done(x, Ok(Attr::Done));
        }
        Err(e) if e.code != "unknown_method" => done(x, Err(err_text(&e))),
        Err(_) => {
            x.reload_active(150);
            done(x, Ok(if hidden_fallback && has_hidden { Attr::RenameHidden } else { Attr::Done }));
        }
    });
}

/// Eigenschappen (Alt+Enter): of the items, else of the folder or the Prullenbak shown.
pub fn properties(x: &Explorer, items: Vec<Item>) {
    if !items.is_empty() {
        crate::dialogs::show_properties(x, items);
        return;
    }
    match x.loc() {
        Loc::Folder(p) => {
            let name = util::basename(&p);
            let item = json!({ "path": p, "name": if name.is_empty() { "/".into() } else { name }, "isDir": true, "icon": x.icon_name_for(&p) });
            crate::dialogs::show_properties(x, vec![Rc::new(item)]);
        }
        Loc::Trash => crate::dialogs::show_trash_properties(x),
        _ => {}
    }
}

// ============================================================================================ new

fn create(x: &Explorer, method: &str, args: Value) {
    op(x, method, args, |x, r| match r {
        Ok(v) => match text_of(&v, "path") {
            Some(p) => {
                push_step(x, "new", trash_step(p.clone()));
                x.select_after_reload(std::slice::from_ref(&p), Some(p.clone()));
            }
            None => x.reload_active(0),
        },
        Err(e) => show_err(x, &e),
    });
}

/// Nieuw > Map (Ctrl+Shift+N): "Nieuwe map", then the rename box on it.
pub fn new_folder(x: &Explorer) {
    if let Some(dir) = x.cur_dir() {
        create(x, "files.createFolder", json!({ "dir": dir }));
    }
}

/// Nieuw > Tekstdocument: "Nieuw tekstdocument.txt", then the rename box.
pub fn new_text(x: &Explorer) {
    if let Some(dir) = x.cur_dir() {
        create(x, "files.createFile", json!({ "dir": dir, "kind": "text" }));
    }
}

// ============================================================================================ clipboard

fn clip(x: &Explorer, op_name: &str) {
    let paths = paths_of(&x.selected_items());
    if paths.is_empty() {
        return;
    }
    x.call("files.clipboardSet", json!({ "paths": paths, "op": op_name }), |x, r| match r {
        Ok(v) => x.apply_clipboard(&v),
        Err(e) => show_err(x, &e),
    });
}

/// Knippen (Ctrl+X).
pub fn cut(x: &Explorer) {
    clip(x, "cut");
}

/// Kopiëren (Ctrl+C).
pub fn copy(x: &Explorer) {
    clip(x, "copy");
}

/// Plakken (Ctrl+V) into `dest` (None: the folder shown): the job card, "errSome", the pasted items selected.
pub fn paste(x: &Explorer, dest: Option<String>) {
    let Some(dest) = dest.or_else(|| x.cur_dir()) else { return };
    op(x, "files.paste", json!({ "dest": dest }), |x, r| match r {
        Ok(v) => {
            report_failed(x, &v);
            x.load_clipboard();
            let paths: Vec<String> =
                v["paths"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            if paths.is_empty() {
                x.reload_active(0);
            } else {
                x.select_after_reload(&paths, None);
            }
        }
        Err(e) => show_err(x, &e),
    });
}

// ============================================================================================ Prullenbak

/// Verwijderen (Del): the selection to the Prullenbak (in the Prullenbak itself the "trashDelete" hint).
pub fn trash(x: &Explorer) {
    let loc = x.loc();
    if loc == Loc::Trash {
        x.toast(s("trashDelete"), ToastOpts { severity: Some(Severity::Info), timeout: 7000, ..Default::default() });
        return;
    }
    if !(loc.is_fileish() || loc == Loc::Gallery) {
        return;
    }
    trash_paths(x, paths_of(&x.selected_items()));
}

/// Shift+Delete: the WebKit app treated it as Delete (main.js onKey: every Delete went to act.trash; "trash,
/// never permanent delete"), and the host has no permanent delete besides emptying the Prullenbak. So the
/// selection goes to the Prullenbak, where it can be restored.
pub fn delete_permanently(x: &Explorer, items: Vec<Item>) {
    let _ = items;
    trash(x);
}

/// Paths to the Prullenbak (files.trash): "errSome" for failures, the toast "trashed" with Ongedaan maken
/// (restore by the trash names the host gave, else looked up) and the undo step.
pub fn trash_paths(x: &Explorer, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    op(x, "files.trash", json!({ "paths": paths }), move |x, r| match r {
        Ok(res) => {
            let bad: HashSet<String> = failed(&res).iter().map(failed_path).collect();
            let done: Vec<String> = paths.iter().filter(|p| !bad.contains(*p)).cloned().collect();
            if !bad.is_empty() {
                x.show_error(&f("errSome", &[("names", &failed_names(&res))]), Some(s("errTitle")));
            }
            if !done.is_empty() && !dry(&res) {
                let known: Option<Vec<String>> = res["trashed"].as_array().filter(|a| !a.is_empty()).map(|a| {
                    a.iter()
                        .filter_map(|t| [st(t, "name"), st(t, "trashName")].into_iter().find(|n| !n.is_empty()))
                        .map(str::to_owned)
                        .collect()
                });
                let step = restore_step(known, done.clone());
                push_step(x, "trash", step.clone());
                let title = n("trashed", done.len(), &[("name", &util::basename(&done[0]))]);
                x.toast(&title, ToastOpts { action: undo_action(x, step), ..Default::default() });
            }
            x.load_trash_info();
            x.reload_active(0);
        }
        Err(e) => show_err(x, &e),
    });
}

/// Undo of a trash: restore by the known trash names, else look them up.
fn restore_step(known: Option<Vec<String>>, paths: Vec<String>) -> Step {
    Rc::new(move |x, done| {
        let go = |x: &Explorer, names: Vec<String>, done: StepDone| {
            if !names.is_empty() {
                restore_names(x, names);
            }
            done(x, Ok(()));
        };
        match known.clone() {
            Some(names) => go(x, names, done),
            None => trash_names_for(x, paths.clone(), move |x, names| go(x, names, done)),
        }
    })
}

/// The trash names of the most recently trashed items that came from these paths (files.list trash:///).
fn trash_names_for(x: &Explorer, paths: Vec<String>, done: impl FnOnce(&Explorer, Vec<String>) + 'static) {
    let tp = x.place_path("trash");
    if tp.is_empty() {
        done(x, Vec::new());
        return;
    }
    x.call("files.list", json!({ "path": tp }), move |x, r| {
        let items = r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
        let names = paths
            .iter()
            .filter_map(|p| {
                items
                    .iter()
                    .filter(|it| {
                        let orig = match st(it, "originalPath") {
                            "" => st(it, "origPath"),
                            o => o,
                        };
                        orig == p
                    })
                    .max_by(|a, b| num(a, "deletedAt").partial_cmp(&num(b, "deletedAt")).unwrap_or(std::cmp::Ordering::Equal))
                    .and_then(|c| [st(c, "trashName"), st(c, "name")].into_iter().find(|n| !n.is_empty()))
                    .map(str::to_owned)
            })
            .collect();
        done(x, names);
    });
}

/// files.restoreTrash {names}: "errSome", else the toast "restored".
fn restore_names(x: &Explorer, names: Vec<String>) {
    op(x, "files.restoreTrash", json!({ "names": names }), move |x, r| match r {
        Ok(v) => {
            if failed(&v).is_empty() {
                let title = n("restored", names.len(), &[("name", names.first().map(String::as_str).unwrap_or(""))]);
                x.toast(&title, ToastOpts { severity: Some(Severity::Success), timeout: 3000, ..Default::default() });
            } else {
                report_failed(x, &v);
            }
            x.load_trash_info();
            x.reload_active(0);
        }
        Err(e) => show_err(x, &e),
    });
}

/// Terugzetten: these Prullenbak items (None: the selection, else all) back where they came from.
pub fn restore(x: &Explorer, items: Option<Vec<Item>>) {
    if x.loc() != Loc::Trash {
        return;
    }
    let list = items.unwrap_or_else(|| {
        let sel = x.selected_items();
        if sel.is_empty() { x.active_items() } else { sel }
    });
    let names: Vec<String> = list
        .iter()
        .filter_map(|i| [st(i, "trashName"), st(i, "name")].into_iter().find(|n| !n.is_empty()))
        .map(str::to_owned)
        .collect();
    if !names.is_empty() {
        restore_names(x, names);
    }
}

/// Prullenbak leegmaken: "Prullenbak leegmaken?" with the count and size (files.trashInfo), then
/// files.emptyTrash and "De Prullenbak is leeggemaakt".
pub fn empty_trash(x: &Explorer) {
    x.call("files.trashInfo", json!({}), |x, r| {
        let info = r.ok();
        let count = info
            .as_ref()
            .and_then(|i| i["count"].as_f64())
            .map(|c| c.max(0.0) as usize)
            .unwrap_or_else(|| x.active_items().len());
        if count == 0 {
            return;
        }
        let size = info.as_ref().map(|i| num(i, "size")).unwrap_or(0.0);
        let msg = n("emptyMsg", count, &[("size", &kit::format::bytes(size))]);
        let w = x.weak();
        Dialog::new(s("emptyTitle"))
            .text(&msg)
            .icon("warning")
            .primary(s("emptyConfirm"))
            .close_label(s("cancel"))
            .danger()
            .show(&x.anchor(), move |r| {
                let Some(x) = w.upgrade() else { return };
                if r != DialogResult::Primary {
                    return;
                }
                op(&x, "files.emptyTrash", json!({}), |x, r| match r {
                    Ok(_) => {
                        x.toast(s("emptied"), ToastOpts { severity: Some(Severity::Success), timeout: 3000, ..Default::default() });
                        x.load_trash_info();
                        x.reload_active(0);
                    }
                    Err(e) => show_err(x, &e),
                });
            });
    });
}

// ============================================================================================ archives

/// Comprimeren tot ZIP-bestand: the job card, the toast "compressed", the zip selected (and renamed when it
/// is in the folder shown).
pub fn compress(x: &Explorer, items: Vec<Item>) {
    let paths = paths_of(&items);
    if paths.is_empty() {
        return;
    }
    op(x, "files.compress", json!({ "paths": paths }), |x, r| match r {
        Ok(v) => {
            if let Some(p) = text_of(&v, "path") {
                x.toast(&f("compressed", &[("name", &util::basename(&p))]), ToastOpts { timeout: 3000, ..Default::default() });
                let rename = (x.cur_dir().as_deref() == Some(util::dirname(&p).as_str())).then(|| p.clone());
                x.select_after_reload(std::slice::from_ref(&p), rename);
            }
        }
        Err(e) => show_err(x, &e),
    });
}

/// Alles uitpakken: the job card, the toast "extracted" with Openen.
pub fn extract(x: &Explorer, item: &Item) {
    op(x, "files.extract", json!({ "path": st(item, "path") }), |x, r| match r {
        Ok(v) => {
            if let Some(p) = text_of(&v, "path") {
                let w = x.weak();
                let target = p.clone();
                let open: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(x) = w.upgrade() {
                        x.navigate(Loc::Folder(target.clone()), NavOpts::default());
                    }
                });
                x.toast(&f("extracted", &[("name", &util::basename(&p))]), ToastOpts { action: Some((s("open").to_owned(), open)), ..Default::default() });
                x.select_after_reload(std::slice::from_ref(&p), None);
            }
        }
        Err(e) => show_err(x, &e),
    });
}

// ============================================================================================ open with

/// The "Openen met" submenu of a file: apps.forFile, each app with its icon; when it takes longer than 450 ms
/// the menu opens with "Bezig met laden..." (main.js loadOpenWith), "Geen apps gevonden" when there are none.
pub fn open_with_items(x: &Explorer, item: &Item, done: impl FnOnce(Vec<MenuItem>) + 'static) {
    type Done = Box<dyn FnOnce(Vec<MenuItem>)>;
    let slot: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let late = slot.clone();
    glib::timeout_add_local_once(Duration::from_millis(450), move || {
        let d = late.borrow_mut().take();
        if let Some(d) = d {
            d(vec![MenuItem::new(s("loading"), || {}).enabled(false)]);
        }
    });
    let path = st(item, "path").to_owned();
    x.call("apps.forFile", json!({ "path": path }), move |x, r| {
        let Some(d) = slot.borrow_mut().take() else { return };
        let apps = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        if apps.is_empty() {
            d(vec![MenuItem::new(s("noApps"), || {}).enabled(false)]);
            return;
        }
        let items = apps
            .iter()
            .map(|a| {
                let (w, p, id) = (x.weak(), path.clone(), st(a, "id").to_owned());
                let label = match st(a, "name") {
                    "" => id.clone(),
                    n => n.to_owned(),
                };
                let icon = st(a, "icon").to_owned();
                MenuItem::new(&label, move || {
                    if let Some(x) = w.upgrade() {
                        op(&x, "files.openWith", json!({ "path": p, "appId": id }), |x, r| {
                            if let Err(e) = r {
                                show_err(x, &e);
                            }
                        });
                    }
                })
                .app_icon(&icon)
            })
            .collect();
        d(items);
    });
}

// ============================================================================================ drop

fn simple_move_undoable(paths: &[String], dest: &str, result: &Value) -> bool {
    if result["undoable"] == false || result["cancelled"] == true || !failed(result).is_empty()
        || ["skipped", "mergedPaths", "overwrittenPaths"].iter().any(|key| result[*key].as_array().is_some_and(|a| !a.is_empty()))
    { return false; }
    let Some(completed) = result["paths"].as_array() else { return false };
    completed.len() == paths.len() && paths.iter().zip(completed).all(|(source, target)| {
        target.as_str().is_some_and(|target| target == util::join_path(dest, &util::basename(source)))
    })
}

/// A drop (main.js dropTo): copy or move `paths` into `dest` (files.copy / files.move) with the job card,
/// "errSome", the toast "copied" / "moved" (with Ongedaan maken for a move: the items back per folder) and
/// the undo step.
pub fn drop_to(x: &Explorer, paths: Vec<String>, dest: String, copy: bool) {
    let paths: Vec<String> =
        paths.into_iter().filter(|p| !p.is_empty() && *p != dest && !util::is_under(&dest, p) && (copy || util::dirname(p) != dest)).collect();
    if paths.is_empty() {
        return;
    }
    let method = if copy { "files.copy" } else { "files.move" };
    let d = dest.clone();
    let sent = paths.clone();
    op(x, method, json!({ "paths": paths, "dest": dest }), move |x, r| match r {
        Ok(res) => {
            report_failed(x, &res);
            if !dry(&res) {
                let count = res["paths"].as_array().map(Vec::len).unwrap_or_else(|| sent.len().saturating_sub(failed(&res).len()));
                if count > 0 {
                    let mut action = None;
                    if !copy && simple_move_undoable(&sent, &d, &res) {
                        let mut by_dir: Vec<(String, Vec<String>)> = Vec::new();
                        for p in &sent {
                            let from = util::dirname(p);
                            let now = util::join_path(&d, &util::basename(p));
                            match by_dir.iter_mut().find(|(dir, _)| *dir == from) {
                                Some((_, list)) => list.push(now),
                                None => by_dir.push((from, vec![now])),
                            }
                        }
                        let step: Step = Rc::new(move |x, done| move_back(x, by_dir.clone(), done));
                        push_step(x, "move", step.clone());
                        action = undo_action(x, step);
                    }
                    let title = n(
                        if copy { "copied" } else { "moved" },
                        count,
                        &[("name", &util::basename(&sent[0])), ("dest", &x.folder_label(&d))],
                    );
                    x.toast(&title, ToastOpts { action, ..Default::default() });
                }
            }
            x.invalidate_dir(&d);
            x.reload_active(0);
        }
        Err(e) => show_err(x, &e),
    });
}

/// Undo of a move: each folder's items back, one after the other (errors shown, the rest goes on).
fn move_back(x: &Explorer, mut rest: Vec<(String, Vec<String>)>, done: StepDone) {
    if rest.is_empty() {
        x.reload_active(0);
        done(x, Ok(()));
        return;
    }
    let (dir, paths) = rest.remove(0);
    op(x, "files.move", json!({ "paths": paths, "dest": dir }), move |x, r| {
        if let Err(e) = r {
            show_err(x, &e);
        }
        move_back(x, rest, done);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_name_list() {
        let res = json!({ "failed": ["/a/x", { "path": "/b/y", "error": "e" }, { "name": "z" }, "/c/1", "/c/2", "/c/3"] });
        assert_eq!(failed_names(&res), "x, y, z, 1, 2 \u{2026}");
        assert_eq!(failed_names(&json!({})), "");
        assert_eq!(job_kind("files.paste"), Some("paste"));
        assert_eq!(job_kind("files.trash"), None);
    }

    #[test]
    fn cancellation_cleanup_warning_stays_visible_with_the_preserved_destination_name() {
        let message = failure_message(&json!({"cancelled": true, "failed": [
            {"path": "/private/dest/report.txt", "code": "partial_cleanup", "error": "The changed partial copy was preserved."}
        ]}));
        assert!(message.contains("report.txt"));
        assert!(message.contains("The changed partial copy was preserved."));
    }

    #[test]
    fn late_callback_cannot_finish_a_retired_or_replaced_operation() {
        let old = Rc::new(());
        assert!(same_operation(Some(old.clone()), &old));
        // Once crash cleanup removed the job, its old HTTP callback has no operation to finish again.
        assert!(!same_operation(None, &old));
        let replacement = Rc::new(());
        assert!(!same_operation(Some(replacement.clone()), &old));
        assert!(same_operation(Some(replacement.clone()), &replacement));
    }

    #[test]
    fn old_move_undo_is_never_offered_for_replacements_merges_skips_or_renamed_targets() {
        let sent = vec!["/source/a.txt".to_owned()];
        assert!(simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a.txt"]})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a.txt"], "undoable": false})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a.txt"], "mergedPaths": ["/dest/a.txt"]})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a.txt"], "overwrittenPaths": ["/dest/a.txt"]})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a (2).txt"]})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": [], "skipped": ["/source/a.txt"]})));
        assert!(!simple_move_undoable(&sent, "/dest", &json!({"paths": ["/dest/a.txt"], "cancelled": true})));
    }
}
