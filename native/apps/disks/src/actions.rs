//! What the buttons and menus do (vendor/ui/js/apps/disks/actions.js): dialogs in Windows wording and calls to
//! disks.*. The destructive ones (format, delete, initialise) say that data is lost and need the volume's (or
//! disk's) name typed before the red button works; Enter in those dialogs means Cancel, and while the operation
//! runs Escape does not close them. The host checks again: system volumes are refused there too, and
//! `expect {device, size}` makes sure the disk did not change meanwhile.
use crate::app::App;
use crate::model::{self, arr, num, st, truthy};
use crate::strings::{s, tf, tp};
use crate::widgets::{Field, hbox, label, radio_desc, vbox};
use kit::controls::{self, Dialog, DialogResult, Dropdown, DropdownOption, InfoBar, MenuItem, ProgressBar, Severity, Slider, Toast};
use kit::prelude::*;
use kit::{CallError, ListenerId};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const MIN_VOLUME: f64 = 8.0 * 1024.0 * 1024.0;

fn expect_of(o: &Value) -> Value {
    let mut e = json!({ "device": o["device"], "size": o["size"] });
    if truthy(&o["uuid"]) {
        e["uuid"] = o["uuid"].clone();
    }
    e
}

fn toast(app: &App, sev: Severity, title: &str, message: &str, ms: u32) {
    Toast::new(title).message(message).severity(sev).timeout(ms).show(&app.parent());
}

fn dry_toast(app: &App) {
    toast(app, Severity::Info, &s("dryDone"), "", 3500);
}

fn is_dry(r: &Value) -> bool {
    truthy(&r["dryRun"])
}

/// util.js run: the call, or None after an error toast; a dry-run answer gets its note (unless quiet).
fn run(app: &Rc<App>, method: &str, args: Value, quiet: bool, done: impl FnOnce(&Rc<App>, Option<Value>) + 'static) {
    let w = Rc::downgrade(app);
    app.ctx.call(method, args, move |r| {
        let Some(app) = w.upgrade() else { return };
        match r {
            Ok(v) => {
                if is_dry(&v) && !quiet {
                    dry_toast(&app);
                }
                done(&app, Some(if v.is_null() { json!({}) } else { v }));
            }
            Err(e) => {
                toast(&app, Severity::Error, &s("failed"), &e.message, 7000);
                done(&app, None);
            }
        }
    });
}

// ------------------------------------------------------------------------------------------- simple actions

pub fn mount(app: &Rc<App>, v: &Value) {
    let name = st(&v["name"]);
    run(app, "disks.mount", json!({ "id": v["id"] }), false, move |app, r| {
        if let Some(r) = r.filter(|r| !is_dry(r)) {
            toast(app, Severity::Success, &tf("mounted", &[("name", &name)]), &st(&r["mountPoint"]), 3000);
        }
        app.load(false);
    });
}

pub fn unmount(app: &Rc<App>, v: &Value) {
    let name = st(&v["name"]);
    run(app, "disks.unmount", json!({ "id": v["id"] }), false, move |app, r| {
        if r.is_some_and(|r| !is_dry(&r)) {
            toast(app, Severity::Success, &tf("unmounted", &[("name", &name)]), "", 3000);
        }
        app.load(false);
    });
}

pub fn open_in_explorer(app: &Rc<App>, v: &Value) {
    run(app, "disks.open", json!({ "id": v["id"] }), true, |app, _| app.load(false));
}

pub fn lock(app: &Rc<App>, v: &Value) {
    let name = st(&v["name"]);
    run(app, "disks.lock", json!({ "id": v["id"] }), false, move |app, r| {
        if r.is_some_and(|r| !is_dry(&r)) {
            toast(app, Severity::Success, &tf("locked", &[("name", &name)]), "", 3000);
        }
        app.load(false);
    });
}

pub fn safe_remove(app: &Rc<App>, d: &Value) {
    let method = if truthy(&d["canPowerOff"]) || !truthy(&d["ejectable"]) { "disks.powerOff" } else { "disks.eject" };
    let (model_name, id) = (st(&d["model"]), st(&d["id"]));
    run(app, method, json!({ "disk": d["id"] }), false, move |app, r| {
        if r.is_some_and(|r| !is_dry(&r)) {
            toast(app, Severity::Success, &tf("safeRemoved", &[("name", &model_name)]), &s("safeRemovedBody"), 6000);
            app.disk_gone(&id);
        }
        app.load(false);
    });
}

pub fn eject(app: &Rc<App>, d: &Value) {
    let (model_name, id) = (st(&d["model"]), st(&d["id"]));
    run(app, "disks.eject", json!({ "disk": d["id"] }), false, move |app, r| {
        if r.is_some_and(|r| !is_dry(&r)) {
            toast(app, Severity::Success, &tf("ejected", &[("name", &model_name)]), "", 4000);
            app.disk_gone(&id);
        }
        app.load(false);
    });
}

// ------------------------------------------------------------------------------------------- dialog parts

/// An inline error area (an error info bar when shown).
#[derive(Clone)]
struct ErrorSlot {
    root: gtk::Box,
}

impl ErrorSlot {
    fn new() -> ErrorSlot {
        let root = vbox(0, "dk-dlg-error");
        root.set_visible(false);
        ErrorSlot { root }
    }
    fn show(&self, e: &CallError) {
        controls::clear(&self.root);
        let b = InfoBar::new(Severity::Error, &s("failed"), &e.message);
        b.set_closable(false);
        self.root.append(b.widget());
        self.root.set_visible(true);
    }
    fn clear(&self) {
        controls::clear(&self.root);
        self.root.set_visible(false);
    }
}

/// A progress line that follows disks.changed for one object while an operation runs.
#[derive(Clone)]
struct ProgressSlot {
    root: gtk::Box,
    text: gtk::Label,
    bar: ProgressBar,
    object: String,
    listener: Rc<Cell<Option<ListenerId>>>,
}

impl ProgressSlot {
    fn new(object: &str) -> ProgressSlot {
        let root = vbox(8, "dk-dlg-progress");
        let text = label("", "dk-dlg-progress-text", true);
        let bar = ProgressBar::new();
        root.append(&text);
        root.append(bar.widget());
        root.set_visible(false);
        ProgressSlot { root, text, bar, object: object.to_owned(), listener: Rc::new(Cell::new(None)) }
    }

    fn start(&self, app: &App, label_text: &str) {
        self.root.set_visible(true);
        self.text.set_label(label_text);
        self.bar.set_indeterminate(true);
        let (text, bar, obj) = (self.text.downgrade(), self.bar.clone(), self.object.clone());
        let id = app.ctx.on("disks.changed", move |_, snap| {
            for d in arr(&snap["disks"]) {
                let hit = if st(&d["block"]) == obj || st(&d["id"]) == obj {
                    Some(d)
                } else {
                    arr(&d["volumes"]).iter().find(|v| st(&v["id"]) == obj)
                };
                if let Some(h) = hit
                    && truthy(&h["busy"])
                {
                    if let Some(p) = h["busy"]["progress"].as_f64()
                        && p > 0.0
                    {
                        bar.set_fraction(p);
                    }
                    if let Some(t) = text.upgrade() {
                        t.set_label(&model::busy_text(&h["busy"]));
                    }
                }
            }
        });
        if let Some(old) = self.listener.replace(Some(id)) {
            app.ctx.off(old);
        }
    }

    fn stop(&self, app: &App) {
        if let Some(id) = self.listener.take() {
            app.ctx.off(id);
        }
        self.root.set_visible(false);
        self.bar.set_indeterminate(false);
    }
}

/// The typed-name confirmation field; `on_valid(bool)` whenever the text changes.
fn confirm_field(word: &str, disk: bool, on_valid: impl Fn(bool) + 'static) -> (Field, Rc<dyn Fn() -> bool>) {
    let f = Field::new(&tf("confirmType", &[("name", word)]), "", &s(if disk { "confirmTypeDescDisk" } else { "confirmTypeDesc" }));
    f.root.add_css_class("dk-confirm");
    f.entry.set_widget_name("dk-confirm");
    let w = word.to_lowercase();
    let e = f.entry.downgrade();
    let valid: Rc<dyn Fn() -> bool> = Rc::new(move || e.upgrade().is_some_and(|e| e.text().trim().to_lowercase() == w));
    let v2 = valid.clone();
    f.entry.connect_changed(move |_| on_valid(v2()));
    (f, valid)
}

fn filesystems(snap: &Value) -> &[Value] {
    arr(&snap["filesystems"])
}

fn fs_options(snap: &Value) -> Vec<DropdownOption> {
    filesystems(snap)
        .iter()
        .map(|f| {
            let name = st(&f["name"]);
            let ok = truthy(&f["available"]);
            let mut o = DropdownOption::new(&st(&f["id"]), &if ok { name.clone() } else { tf("fsMissing", &[("fs", &name)]) });
            o.disabled = !ok;
            o
        })
        .collect()
}

/// The file-system drop-down with the description of the chosen one below it (Windows lists only names).
fn fs_picker(snap: &Value, value: &str, on_change: impl Fn(&str) + 'static) -> (gtk::Box, Dropdown) {
    let col = vbox(4, "dk-dlg-fs");
    let dd = Dropdown::new(fs_options(snap), Some(value));
    dd.widget().set_hexpand(true);
    dd.widget().set_widget_name("dk-fs");
    let desc = label("", "dk-fs-desc", true);
    let snap2 = snap.clone();
    let show = move |l: &gtk::Label, id: &str| {
        let d = filesystems(&snap2).iter().find(|f| st(&f["id"]) == id).map(|f| st(&f["description"])).unwrap_or_default();
        l.set_label(&d);
        l.set_visible(!d.is_empty());
    };
    show(&desc, value);
    let dl = desc.downgrade();
    dd.connect_changed(move |x| {
        if let Some(l) = dl.upgrade() {
            show(&l, x);
        }
        on_change(x);
    });
    col.append(&controls::field(&s("fsField"), dd.widget(), None));
    col.append(&desc);
    (col, dd)
}

fn default_fs(snap: &Value, disk: &Value) -> String {
    let ok = |id: &str| filesystems(snap).iter().any(|f| f["id"] == id && truthy(&f["available"]));
    let removable = truthy(&disk["removable"]) || disk["kind"] == "usb" || disk["kind"] == "sd";
    let order: &[&str] = if removable { &["exfat", "ntfs", "vfat", "ext4"] } else { &["ext4", "ntfs", "exfat"] };
    for id in order {
        if ok(id) {
            return (*id).to_owned();
        }
    }
    filesystems(snap).iter().find(|f| truthy(&f["available"])).map(|f| st(&f["id"])).unwrap_or_else(|| "ext4".into())
}

fn label_limit(snap: &Value, fs: &str) -> i32 {
    filesystems(snap).iter().find(|f| st(&f["id"]) == fs).map(|f| num(&f["maxLabel"]) as i32).unwrap_or(32)
}

/// The password pair (with its checks).
#[derive(Clone)]
struct Passwords {
    root: gtk::Grid,
    p1: Field,
    p2: Field,
}

impl Passwords {
    fn new(on_change: Rc<dyn Fn()>) -> Passwords {
        let p1 = Field::password(&s("pw1"));
        let p2 = Field::password(&s("pw2"));
        p1.entry.set_widget_name("dk-pw1");
        p2.entry.set_widget_name("dk-pw2");
        let root = gtk::Grid::new();
        root.add_css_class("dk-dlg-pw");
        root.set_column_spacing(12);
        root.set_column_homogeneous(true);
        root.set_margin_start(28);
        root.attach(&p1.root, 0, 0, 1, 1);
        root.attach(&p2.root, 1, 0, 1, 1);
        p1.root.set_valign(gtk::Align::Start);
        p2.root.set_valign(gtk::Align::Start);
        root.set_visible(false);
        let c = on_change.clone();
        p1.entry.connect_changed(move |_| c());
        p2.entry.connect_changed(move |_| on_change());
        Passwords { root, p1, p2 }
    }
    fn value(&self) -> String {
        self.p1.text()
    }
    fn problem(&self) -> String {
        if self.p1.text().chars().count() < 8 {
            s("pwShort")
        } else if self.p1.text() != self.p2.text() {
            s("pwMismatch")
        } else {
            String::new()
        }
    }
}

/// Two columns side by side (`.dk-dlg__grid`).
fn two_columns(a: &impl IsA<gtk::Widget>, b: &impl IsA<gtk::Widget>) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("dk-dlg-grid");
    g.set_column_spacing(12);
    g.set_column_homogeneous(true);
    a.as_ref().set_valign(gtk::Align::Start);
    b.as_ref().set_valign(gtk::Align::Start);
    g.attach(a, 0, 0, 1, 1);
    g.attach(b, 1, 0, 1, 1);
    g
}

fn text_p(text: &str) -> gtk::Label {
    let l = label(text, "dk-dlg-text", true);
    l.set_max_width_chars(60);
    l
}

fn warning(msg: &str) -> gtk::Widget {
    let b = InfoBar::new(Severity::Warning, "", msg);
    b.set_closable(false);
    b.widget().clone().upcast()
}

/// The body at most `width` px wide (the dialog's CSS width less its padding and border), so wrapping text
/// wraps there instead of widening the dialog (components.css .fl-dialog max-width 548 px).
fn body(content: &gtk::Box, width: i32) -> crate::widgets::Clamp {
    let c = crate::widgets::Clamp::new(width - 50);
    c.set_child(content);
    c
}

type Holder = Rc<RefCell<Option<Dialog>>>;

/// Show one of our dialogs: kept in `holder` while open (its validators reach it there); when it closes, the
/// page reloads and a render it held back runs.
fn present(app: &Rc<App>, dlg: Dialog, holder: &Holder, after: impl FnOnce() + 'static) {
    *holder.borrow_mut() = Some(dlg.clone());
    let (w, h) = (Rc::downgrade(app), holder.clone());
    dlg.show(&app.parent(), move |_| {
        h.borrow_mut().take();
        after();
        if let Some(app) = w.upgrade() {
            app.load(false);
            app.dialog_closed();
        }
    });
}

fn with_dialog(holder: &Holder, f: impl FnOnce(&Dialog)) {
    let d = holder.borrow().clone();
    if let Some(d) = d {
        f(&d);
    }
}

fn the_app(w: &Weak<App>) -> Option<Rc<App>> {
    w.upgrade()
}

// ------------------------------------------------------------------------------------------- dialogs

pub fn change_label(app: &Rc<App>, v: &Value) {
    let snap = app.snap().unwrap_or(Value::Null);
    let fs = st(&v["fsType"]);
    let limit = label_limit(&snap, &fs);
    let err = ErrorSlot::new();
    let hint = if fs == "vfat" { s("labelHintFat") } else { tp("labelHint", limit as f64, &[("n", &limit.to_string())]) };
    let tb = Field::new(&s("labelField"), &st(&v["label"]), &hint);
    tb.entry.set_max_length(limit);
    tb.entry.set_widget_name("dk-label-field");
    if fs == "vfat" {
        // text-transform: uppercase (the host writes FAT labels in capitals anyway)
        let busy = Rc::new(Cell::new(false));
        tb.entry.connect_changed(move |e| {
            if busy.get() {
                return;
            }
            let t = e.text().to_string();
            let up = t.to_uppercase();
            if up != t && up.chars().count() == t.chars().count() {
                busy.set(true);
                let pos = e.position();
                e.set_text(&up);
                e.set_position(pos);
                busy.set(false);
            }
        });
    }
    let content = vbox(14, "dk-dlg");
    content.append(&tb.root);
    if truthy(&v["labelRemount"]) {
        let n = InfoBar::new(Severity::Info, "", &s("labelRemount"));
        n.set_closable(false);
        content.append(n.widget());
    }
    content.append(&err.root);
    let holder: Holder = Rc::new(RefCell::new(None));
    let (w, id, entry, err2) = (Rc::downgrade(app), v["id"].clone(), tb.entry.clone(), err.clone());
    let dlg = Dialog::new(&s("labelTitle")).body(&body(&content, 548)).primary(&s("save")).close_label(&s("cancel")).on_primary(move |dlg| {
        let Some(app) = the_app(&w) else { return true };
        err2.clear();
        dlg.set_busy(true);
        let (dlg, err, w) = (dlg.clone(), err2.clone(), w.clone());
        app.ctx.call("disks.setLabel", json!({ "id": id, "label": entry.text().to_string() }), move |r| {
            let Some(app) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    if is_dry(&r) {
                        dry_toast(&app);
                    } else {
                        let me = st(&r["mountError"]);
                        let sev = if me.is_empty() { Severity::Success } else { Severity::Warning };
                        toast(&app, sev, &s("labelDone"), &me, if me.is_empty() { 3000 } else { 7000 });
                    }
                    dlg.close(DialogResult::Primary);
                }
                Err(e) => {
                    dlg.set_busy(false);
                    err.show(&e);
                }
            }
        });
        false
    });
    present(app, dlg, &holder, || {});
    let e = tb.entry.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(50), move || {
        if let Some(e) = e.upgrade() {
            e.grab_focus();
            e.select_region(0, -1);
        }
    });
}

/// Windows' "Formatteren" dialog with the typed-name confirmation (and optional LUKS).
pub fn format(app: &Rc<App>, d: &Value, v: &Value) {
    let snap = app.snap().unwrap_or(Value::Null);
    let word = model::confirm_word(v);
    let own = st(&v["fsType"]);
    let fs = Rc::new(RefCell::new(
        if filesystems(&snap).iter().any(|f| st(&f["id"]) == own && truthy(&f["available"])) { own.clone() } else { default_fs(&snap, d) },
    ));
    let err = ErrorSlot::new();
    let prog = ProgressSlot::new(&st(&v["id"]));
    let initial_label = if truthy(&v["label"]) { st(&v["label"]) } else if truthy(&v["fsType"]) { String::new() } else { s("newVolumeLabel") };
    let label_box = Field::new(&s("labelField"), &initial_label, "");
    label_box.entry.set_max_length(label_limit(&snap, &fs.borrow()));
    label_box.entry.set_widget_name("dk-format-label");
    let holder: Holder = Rc::new(RefCell::new(None));
    let typed_ok = Rc::new(Cell::new(false));
    let validate_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let revalidate: Rc<dyn Fn()> = {
        let vs = validate_slot.clone();
        Rc::new(move || {
            let f = vs.borrow().clone();
            if let Some(f) = f {
                f();
            }
        })
    };
    let pw = Passwords::new(revalidate.clone());
    let quick = controls::checkbox_desc(&s("quick"), &s("quickDesc"));
    quick.set_active(true);
    quick.set_widget_name("dk-quick");
    let enc = controls::checkbox_desc(&s("encrypt"), &s("encryptDesc"));
    enc.set_widget_name("dk-encrypt");
    enc.set_visible(truthy(&snap["encryption"]["supported"]));
    let (pw_root, rv) = (pw.root.downgrade(), revalidate.clone());
    enc.connect_toggled(move |c| {
        if let Some(p) = pw_root.upgrade() {
            p.set_visible(c.is_active());
        }
        rv();
    });
    let (fs2, lb, snap2, rv) = (fs.clone(), label_box.entry.downgrade(), snap.clone(), revalidate.clone());
    let (fs_col, _dd) = fs_picker(&snap, &fs.borrow(), move |x| {
        *fs2.borrow_mut() = x.to_owned();
        if let Some(e) = lb.upgrade() {
            e.set_max_length(label_limit(&snap2, x));
        }
        rv();
    });
    let (to, rv) = (typed_ok.clone(), revalidate.clone());
    let (confirm, confirm_valid) = confirm_field(&word, false, move |ok| {
        to.set(ok);
        rv();
    });
    {
        let (h, to, enc, pw) = (holder.clone(), typed_ok.clone(), enc.downgrade(), pw.clone());
        *validate_slot.borrow_mut() = Some(Rc::new(move || {
            let on = enc.upgrade().is_some_and(|e| e.is_active() && e.is_visible());
            let problem = if on { pw.problem() } else { String::new() };
            pw.p2.set_error(if on && !pw.p2.text().is_empty() { &problem } else { "" });
            let ok = to.get() && problem.is_empty();
            with_dialog(&h, |d| d.set_primary_enabled(ok));
        }));
    }
    let content = vbox(14, "dk-dlg");
    content.append(&warning(&s("formatWarn")));
    content.append(&two_columns(&label_box.root, &fs_col));
    content.append(&quick);
    content.append(&enc);
    content.append(&pw.root);
    content.append(&confirm.root);
    content.append(&err.root);
    content.append(&prog.root);
    let w = Rc::downgrade(app);
    let (vv, dd, fs3, lb, quick2, enc2, pw2, err2, prog2, rv) =
        (v.clone(), d.clone(), fs.clone(), label_box.entry.clone(), quick.clone(), enc.clone(), pw.clone(), err.clone(), prog.clone(), revalidate.clone());
    let dlg = Dialog::new(&tf("formatTitle", &[("name", &st(&v["name"]))]))
        .body(&body(&content, 520))
        .primary(&s("formatBtn"))
        .close_label(&s("cancel"))
        .danger()
        .width(520)
        .on_primary(move |dlg| {
            let Some(app) = the_app(&w) else { return true };
            if !confirm_valid() {
                return false;
            }
            err2.clear();
            prog2.start(&app, &s("formatting"));
            dlg.set_busy(true);
            let label_text = lb.text().to_string();
            let mut args = json!({
                "id": vv["id"], "fsType": fs3.borrow().clone(), "label": label_text, "quick": quick2.is_active(),
                "expect": expect_of(&vv), "mount": truthy(&vv["mounted"]) || truthy(&dd["removable"]),
            });
            if enc2.is_active() && enc2.is_visible() {
                args["encrypt"] = json!({ "passphrase": pw2.value() });
            }
            let (dlg, err, prog, w, rv, name) = (dlg.clone(), err2.clone(), prog2.clone(), w.clone(), rv.clone(), st(&vv["name"]));
            app.ctx.call("disks.format", args, move |r| {
                let Some(app) = w.upgrade() else { return };
                prog.stop(&app);
                match r {
                    Ok(r) => {
                        if is_dry(&r) {
                            dry_toast(&app);
                        } else {
                            let n = if label_text.is_empty() { name } else { label_text };
                            toast(&app, Severity::Success, &tf("formatDone", &[("name", &n)]), &st(&r["mountError"]), 4000);
                        }
                        dlg.close(DialogResult::Primary);
                    }
                    Err(e) => {
                        dlg.set_busy(false);
                        err.show(&e);
                        rv();
                    }
                }
            });
            false
        });
    let (a, p, vs) = (Rc::downgrade(app), prog.clone(), validate_slot.clone());
    present(app, dlg, &holder, move || {
        // the validator holds the dialog's fields; without it the signal handlers keep no widget alive
        vs.borrow_mut().take();
        if let Some(app) = a.upgrade() {
            p.stop(&app);
        }
    });
    revalidate();
}

pub fn delete_volume(app: &Rc<App>, d: &Value, v: &Value) {
    let word = model::confirm_word(v);
    let err = ErrorSlot::new();
    let prog = ProgressSlot::new(&st(&v["id"]));
    let holder: Holder = Rc::new(RefCell::new(None));
    let h = holder.clone();
    let (confirm, confirm_valid) = confirm_field(&word, false, move |ok| with_dialog(&h, |d| d.set_primary_enabled(ok)));
    let content = vbox(14, "dk-dlg");
    content.append(&text_p(&s("deleteWarn")));
    if truthy(&d["dynamic"]) {
        let b = InfoBar::new(Severity::Warning, &s("dynamicTitle"), &s("dynamicBody"));
        b.set_closable(false);
        content.append(b.widget());
    }
    content.append(&confirm.root);
    content.append(&err.root);
    content.append(&prog.root);
    let w = Rc::downgrade(app);
    let (vv, err2, prog2, cv) = (v.clone(), err.clone(), prog.clone(), confirm_valid.clone());
    let dlg = Dialog::new(&tf("deleteTitle", &[("name", &st(&v["name"]))]))
        .body(&body(&content, 500))
        .primary(&s("deleteBtn"))
        .close_label(&s("cancel"))
        .danger()
        .width(500)
        .on_primary(move |dlg| {
            let Some(app) = the_app(&w) else { return true };
            if !confirm_valid() {
                return false;
            }
            err2.clear();
            prog2.start(&app, &s("op_delete"));
            dlg.set_busy(true);
            let (dlg, err, prog, w, id, cv) = (dlg.clone(), err2.clone(), prog2.clone(), w.clone(), st(&vv["id"]), cv.clone());
            app.ctx.call("disks.deletePartition", json!({ "id": vv["id"], "expect": expect_of(&vv) }), move |r| {
                let Some(app) = w.upgrade() else { return };
                prog.stop(&app);
                match r {
                    Ok(r) => {
                        let dry = is_dry(&r);
                        toast(&app, if dry { Severity::Info } else { Severity::Success }, &s(if dry { "dryDone" } else { "deleteDone" }), "", 3500);
                        dlg.close(DialogResult::Primary);
                        if !dry {
                            app.volume_gone(&id);
                        }
                    }
                    Err(e) => {
                        dlg.set_busy(false);
                        dlg.set_primary_enabled(cv());
                        err.show(&e);
                    }
                }
            });
            false
        });
    let (a, p) = (Rc::downgrade(app), prog.clone());
    present(app, dlg, &holder, move || {
        if let Some(app) = a.upgrade() {
            p.stop(&app);
        }
    });
    with_dialog(&holder, |d| d.set_primary_enabled(false));
}

/// "Nieuw eenvoudig volume" in a free region.
pub fn new_volume(app: &Rc<App>, d: &Value, region: &Value) {
    let snap = app.snap().unwrap_or(Value::Null);
    let fs = Rc::new(RefCell::new(default_fs(&snap, d)));
    let size = num(&region["size"]);
    let max_gb = size / GB;
    let err = ErrorSlot::new();
    let prog = ProgressSlot::new(&st(&d["block"]));
    let round = |x: f64| ((x * 100.0).floor() / 100.0).max(0.01);
    let size_gb = Rc::new(Cell::new(round(max_gb)));
    let holder: Holder = Rc::new(RefCell::new(None));
    let validate_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let revalidate: Rc<dyn Fn()> = {
        let vs = validate_slot.clone();
        Rc::new(move || {
            let f = vs.borrow().clone();
            if let Some(f) = f {
                f();
            }
        })
    };
    let max_text = format!("{} GB", kit::format::number(size_gb.get(), 2));
    let size_box = Field::new(&s("sizeField"), &kit::format::number(size_gb.get(), 2), &tf("sizeHint", &[("max", &max_text)]));
    size_box.root.set_size_request(140, -1);
    size_box.entry.set_width_chars(4);
    size_box.entry.set_widget_name("dk-size");
    let slider = Slider::new(1.0, 1000.0, 1.0, 1000.0);
    slider.set_format(move |pm| kit::format::bytes(pm / 1000.0 * size));
    // components.js shows the value as a tip on the thumb, not as text beside the slider
    slider.set_value_visible(false);
    slider.scale().set_tooltip_text(Some(&kit::format::bytes(size)));
    slider.widget().set_hexpand(true);
    slider.widget().set_valign(gtk::Align::Center);
    slider.widget().set_margin_top(18);
    slider.scale().set_widget_name("dk-size-slider");
    let silent = Rc::new(Cell::new(false));
    {
        let (sg, sl, rv, silent) = (size_gb.clone(), slider.clone(), revalidate.clone(), silent.clone());
        size_box.entry.connect_changed(move |e| {
            if silent.get() {
                return;
            }
            let n = model::parse_locale_number(&e.text());
            sg.set(if n.is_finite() && n > 0.0 { n.min(max_gb) } else { 0.0 });
            sl.set_value(if max_gb > 0.0 { (sg.get() / max_gb * 1000.0).round() } else { 0.0 });
            rv();
        });
    }
    {
        let (sg, sb, rv, silent) = (size_gb.clone(), size_box.entry.downgrade(), revalidate.clone(), silent.clone());
        let tip = slider.scale().downgrade();
        slider.connect_changed(move |pm| {
            if let Some(t) = tip.upgrade() {
                t.set_tooltip_text(Some(&kit::format::bytes(pm / 1000.0 * size)));
            }
            sg.set(round(pm / 1000.0 * max_gb));
            if let Some(e) = sb.upgrade() {
                silent.set(true);
                e.set_text(&kit::format::number(sg.get(), 2));
                silent.set(false);
            }
            rv();
        });
    }
    let label_box = Field::new(&s("labelField"), &s("newVolumeLabel"), "");
    label_box.entry.set_max_length(label_limit(&snap, &fs.borrow()));
    label_box.entry.set_widget_name("dk-new-label");
    let (fs2, lb, snap2) = (fs.clone(), label_box.entry.downgrade(), snap.clone());
    let (fs_col, _dd) = fs_picker(&snap, &fs.borrow(), move |x| {
        *fs2.borrow_mut() = x.to_owned();
        if let Some(e) = lb.upgrade() {
            e.set_max_length(label_limit(&snap2, x));
        }
    });
    let pw = Passwords::new(revalidate.clone());
    let enc = controls::checkbox_desc(&s("encrypt"), &s("encryptDesc"));
    enc.set_widget_name("dk-encrypt");
    enc.set_visible(truthy(&snap["encryption"]["supported"]));
    let (pw_root, rv) = (pw.root.downgrade(), revalidate.clone());
    enc.connect_toggled(move |c| {
        if let Some(p) = pw_root.upgrade() {
            p.set_visible(c.is_active());
        }
        rv();
    });
    {
        let (h, sg, enc, pw) = (holder.clone(), size_gb.clone(), enc.downgrade(), pw.clone());
        *validate_slot.borrow_mut() = Some(Rc::new(move || {
            let on = enc.upgrade().is_some_and(|e| e.is_active() && e.is_visible());
            let problem = if on { pw.problem() } else { String::new() };
            pw.p2.set_error(if on && !pw.p2.text().is_empty() { &problem } else { "" });
            let bad = sg.get() * GB < MIN_VOLUME || !problem.is_empty();
            with_dialog(&h, |d| d.set_primary_enabled(!bad));
        }));
    }
    let disk_text = format!("{} ({})", st(&d["name"]), st(&d["model"]));
    let content = vbox(14, "dk-dlg");
    content.append(&text_p(&tf(if truthy(&region["logical"]) { "newIntroLogical" } else { "newIntro" }, &[("disk", &disk_text)])));
    let size_row = hbox(16, "dk-dlg-size");
    // grid-template-columns: 140px 1fr
    let size_col = crate::widgets::Clamp::new(140);
    size_col.set_hexpand(false);
    size_col.set_child(&size_box.root);
    size_row.append(&size_col);
    size_row.append(slider.widget());
    content.append(&size_row);
    content.append(&two_columns(&label_box.root, &fs_col));
    content.append(&enc);
    content.append(&pw.root);
    content.append(&err.root);
    content.append(&prog.root);
    let w = Rc::downgrade(app);
    let (dd, rr, fs3, lb, enc2, pw2, err2, prog2, sg) =
        (d.clone(), region.clone(), fs.clone(), label_box.entry.clone(), enc.clone(), pw.clone(), err.clone(), prog.clone(), size_gb.clone());
    let dlg = Dialog::new(&s("newTitle")).body(&body(&content, 520)).primary(&s("createBtn")).close_label(&s("cancel")).width(520).on_primary(move |dlg| {
        let Some(app) = the_app(&w) else { return true };
        err2.clear();
        prog2.start(&app, &s("creating"));
        dlg.set_busy(true);
        let bytes_wanted = if sg.get() >= max_gb - 0.005 { 0.0 } else { (sg.get() * GB).round() };
        let label_text = lb.text().to_string();
        let mut args = json!({
            "disk": dd["id"], "offset": rr["offset"], "size": bytes_wanted as i64, "fsType": fs3.borrow().clone(),
            "label": label_text, "expect": expect_of(&dd),
        });
        if enc2.is_active() && enc2.is_visible() {
            args["encrypt"] = json!({ "passphrase": pw2.value() });
        }
        let (dlg, err, prog, w) = (dlg.clone(), err2.clone(), prog2.clone(), w.clone());
        app.ctx.call("disks.createPartition", args, move |r| {
            let Some(app) = w.upgrade() else { return };
            prog.stop(&app);
            match r {
                Ok(r) => {
                    if is_dry(&r) {
                        dry_toast(&app);
                    } else {
                        let n = if label_text.is_empty() { s("newVolumeLabel") } else { label_text };
                        let mp = st(&r["mountPoint"]);
                        let msg = if mp.is_empty() { st(&r["mountError"]) } else { mp };
                        toast(&app, Severity::Success, &tf("createDone", &[("name", &n)]), &msg, 4000);
                    }
                    dlg.close(DialogResult::Primary);
                }
                Err(e) => {
                    dlg.set_busy(false);
                    err.show(&e);
                }
            }
        });
        false
    });
    let (a, p, vs) = (Rc::downgrade(app), prog.clone(), validate_slot.clone());
    present(app, dlg, &holder, move || {
        // the validator holds the dialog's fields; without it the signal handlers keep no widget alive
        vs.borrow_mut().take();
        if let Some(app) = a.upgrade() {
            p.stop(&app);
        }
    });
    revalidate();
}

pub fn unlock(app: &Rc<App>, v: &Value) {
    let err = ErrorSlot::new();
    let pw = Field::password(&s("pw1"));
    pw.entry.set_widget_name("dk-unlock-pw");
    let content = vbox(14, "dk-dlg");
    content.append(&text_p(&s("unlockIntro")));
    content.append(&pw.root);
    content.append(&err.root);
    let holder: Holder = Rc::new(RefCell::new(None));
    let w = Rc::downgrade(app);
    let (vv, entry, err2) = (v.clone(), pw.entry.clone(), err.clone());
    let dlg = Dialog::new(&tf("unlockTitle", &[("name", &st(&v["name"]))]))
        .body(&body(&content, 440))
        .primary(&s("unlockBtn"))
        .close_label(&s("cancel"))
        .width(440)
        .on_primary(move |dlg| {
            let Some(app) = the_app(&w) else { return true };
            err2.clear();
            let pass = entry.text().to_string();
            if pass.is_empty() {
                entry.grab_focus();
                return false;
            }
            dlg.set_busy(true);
            let (dlg, err, entry, w, name) = (dlg.clone(), err2.clone(), entry.clone(), w.clone(), st(&vv["name"]));
            app.ctx.call("disks.unlock", json!({ "id": vv["id"], "passphrase": pass }), move |r| {
                let Some(app) = w.upgrade() else { return };
                match r {
                    Ok(r) => {
                        if is_dry(&r) {
                            dry_toast(&app);
                        } else {
                            toast(&app, Severity::Success, &tf("unlockDone", &[("name", &name)]), &st(&r["mountPoint"]), 3500);
                        }
                        dlg.close(DialogResult::Primary);
                    }
                    Err(e) => {
                        dlg.set_busy(false);
                        err.show(&e);
                        entry.set_text("");
                        entry.grab_focus();
                    }
                }
            });
            false
        });
    present(app, dlg, &holder, || {});
}

pub fn initialize(app: &Rc<App>, d: &Value) {
    let err = ErrorSlot::new();
    let prog = ProgressSlot::new(&st(&d["block"]));
    let table = Rc::new(RefCell::new("gpt".to_owned()));
    let gpt = radio_desc(&s("initGpt"), &s("initGptDesc"), None);
    let dos = radio_desc(&s("initDos"), &s("initDosDesc"), Some(&gpt));
    gpt.set_active(true);
    gpt.set_widget_name("dk-init-gpt");
    dos.set_widget_name("dk-init-dos");
    for (b, value) in [(&gpt, "gpt"), (&dos, "dos")] {
        let t = table.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                *t.borrow_mut() = value.to_owned();
            }
        });
    }
    let radios = vbox(4, "w11-radio-group");
    radios.append(&gpt);
    radios.append(&dos);
    // Always destructive: "no volumes" only means Ubuntu recognises nothing (VeraCrypt, Storage Spaces, a RAID
    // member, a raw image ...). So: the warning, the typed disk name, and Enter = Cancel, also for an "empty" disk.
    let has_data = !arr(&d["volumes"]).is_empty();
    let holder: Holder = Rc::new(RefCell::new(None));
    let h = holder.clone();
    let name = st(&d["name"]);
    let (confirm, confirm_valid) = confirm_field(&name, true, move |ok| with_dialog(&h, |d| d.set_primary_enabled(ok)));
    let content = vbox(14, "dk-dlg");
    content.append(&text_p(&s("initIntro")));
    content.append(&radios);
    content.append(&warning(&if has_data { s("initWarn") } else { format!("{} {}", s("initWarn"), s("initWarnUnknown")) }));
    content.append(&confirm.root);
    content.append(&err.root);
    content.append(&prog.root);
    let w = Rc::downgrade(app);
    let (dd, err2, prog2, cv) = (d.clone(), err.clone(), prog.clone(), confirm_valid.clone());
    let dlg = Dialog::new(&tf("initTitle", &[("disk", &name)]))
        .body(&body(&content, 520))
        .primary(&s("initBtn"))
        .close_label(&s("cancel"))
        .danger()
        .width(520)
        .on_primary(move |dlg| {
            let Some(app) = the_app(&w) else { return true };
            if !confirm_valid() {
                return false;
            }
            err2.clear();
            prog2.start(&app, &s("op_other"));
            dlg.set_busy(true);
            let (dlg, err, prog, w, name, cv) = (dlg.clone(), err2.clone(), prog2.clone(), w.clone(), st(&dd["name"]), cv.clone());
            let args = json!({ "disk": dd["id"], "table": table.borrow().clone(), "expect": expect_of(&dd) });
            app.ctx.call("disks.initialize", args, move |r| {
                let Some(app) = w.upgrade() else { return };
                prog.stop(&app);
                match r {
                    Ok(r) => {
                        let dry = is_dry(&r);
                        let title = if dry { s("dryDone") } else { tf("initDone", &[("disk", &name)]) };
                        toast(&app, if dry { Severity::Info } else { Severity::Success }, &title, "", 3500);
                        dlg.close(DialogResult::Primary);
                    }
                    Err(e) => {
                        dlg.set_busy(false);
                        dlg.set_primary_enabled(cv());
                        err.show(&e);
                    }
                }
            });
            false
        });
    let (a, p) = (Rc::downgrade(app), prog.clone());
    present(app, dlg, &holder, move || {
        if let Some(app) = a.upgrade() {
            p.stop(&app);
        }
    });
    with_dialog(&holder, |d| d.set_primary_enabled(false));
}

// ------------------------------------------------------------------------------------------- menus

fn item(app: &Rc<App>, label: &str, glyph: &str, name: &str, enabled: bool, f: impl Fn(&Rc<App>) + 'static) -> MenuItem {
    let w = Rc::downgrade(app);
    MenuItem::new(label, move || {
        if let Some(app) = w.upgrade() {
            f(&app);
        }
    })
    .glyph(glyph)
    .name(name)
    .enabled(enabled)
}

/// The context menu of a volume (bar segment, row, extended partition).
pub fn volume_menu(app: &Rc<App>, d: &Value, v: &Value) -> Vec<MenuItem> {
    let enc = &v["encrypted"];
    let locked = truthy(enc) && !truthy(&enc["unlocked"]);
    let busy = truthy(&v["busy"]);
    let mut out = Vec::new();
    let vv = v.clone();
    out.push(item(app, &s("open"), "folder-open", "disks-menu-open", !busy && !locked && (truthy(&v["mounted"]) || truthy(&v["canMount"])), move |app| {
        open_in_explorer(app, &vv)
    }));
    let vv = v.clone();
    if truthy(&v["mounted"]) {
        out.push(item(app, &s("unmount"), "external", "disks-menu-unmount", !busy && truthy(&v["canUnmount"]), move |app| unmount(app, &vv)));
    } else {
        out.push(item(app, &s("mount"), "link", "disks-menu-mount", !busy && truthy(&v["canMount"]), move |app| mount(app, &vv)));
    }
    if truthy(enc) {
        let vv = v.clone();
        if locked {
            out.push(item(app, &format!("{}\u{2026}", s("unlock")), "lock-open", "disks-menu-unlock", !busy, move |app| unlock(app, &vv)));
        } else {
            out.push(item(app, &s("lock"), "lock", "disks-menu-lock", !busy && !truthy(&v["isSystem"]), move |app| lock(app, &vv)));
        }
    }
    out.push(MenuItem::separator());
    let vv = v.clone();
    out.push(item(app, &format!("{}\u{2026}", s("changeLabel")), "rename", "disks-menu-label", !busy && truthy(&v["canLabel"]), move |app| {
        change_label(app, &vv)
    }));
    let (dd, vv) = (d.clone(), v.clone());
    out.push(item(app, &format!("{}\u{2026}", s("format")), "storage", "disks-menu-format", !busy && truthy(&v["canFormat"]), move |app| format(app, &dd, &vv)).danger());
    let (dd, vv) = (d.clone(), v.clone());
    out.push(
        item(app, &format!("{}\u{2026}", s("deleteVolume")), "delete", "disks-menu-delete", !busy && truthy(&v["canDelete"]), move |app| delete_volume(app, &dd, &vv))
            .danger(),
    );
    out.push(MenuItem::separator());
    let id = st(&v["id"]);
    out.push(item(app, &s("properties"), "properties", "disks-menu-properties", true, move |app| app.open_volume(&id)));
    out
}

/// May a not-initialised disk be initialised right now?
pub fn can_init(d: &Value) -> bool {
    truthy(&d["canInitialize"]) && !model::disk_busy(d)
}

/// May a free region get a new volume right now?
pub fn can_create_in(d: &Value, region: &Value) -> bool {
    let allowed = if region["canCreate"].is_null() { truthy(&d["canPartition"]) } else { truthy(&region["canCreate"]) };
    allowed && !model::disk_busy(d)
}

pub fn free_menu(app: &Rc<App>, d: &Value, region: &Value) -> Vec<MenuItem> {
    if !truthy(&d["partitionTable"]) {
        let dd = d.clone();
        return vec![item(app, &format!("{}\u{2026}", s("initialize")), "disk", "disks-menu-initialize", can_init(d), move |app| initialize(app, &dd)).danger()];
    }
    let (dd, rr) = (d.clone(), region.clone());
    vec![item(app, &s("newVolumeDots"), "plus", "disks-menu-new", can_create_in(d, region), move |app| new_volume(app, &dd, &rr))]
}

/// Double-click / Enter on a free segment: the first action when it is allowed, else the menu (it says why), at
/// the pointer or below the segment.
pub fn free_default(app: &Rc<App>, d: &Value, region: &Value, anchor: &gtk::Widget, at: Option<(f64, f64)>) {
    let allowed = if truthy(&d["partitionTable"]) { can_create_in(d, region) } else { can_init(d) };
    if allowed {
        if truthy(&d["partitionTable"]) {
            new_volume(app, d, region);
        } else {
            initialize(app, d);
        }
        return;
    }
    let items = free_menu(app, d, region);
    match at {
        Some((x, y)) => app.menu_at(anchor, x, y, items),
        None => app.menu_at(anchor, 8.0, anchor.height() as f64, items),
    }
}

pub fn disk_menu(app: &Rc<App>, d: &Value) -> Vec<MenuItem> {
    let busy = model::disk_busy(d);
    let mut out = Vec::new();
    if truthy(&d["removable"]) {
        let dd = d.clone();
        out.push(item(app, &s("safeRemove"), "usb", "disks-menu-saferemove", !busy && truthy(&d["canEject"]), move |app| safe_remove(app, &dd)));
    }
    if truthy(&d["mediaRemovable"]) || d["kind"] == "optical" {
        let dd = d.clone();
        out.push(item(app, &s("eject"), "arrow-up", "disks-menu-eject", !busy && truthy(&d["canEject"]), move |app| eject(app, &dd)));
    }
    if d["kind"] != "optical" {
        let dd = d.clone();
        let text = format!("{}\u{2026}", if truthy(&d["partitionTable"]) { s("reinitialize") } else { s("initialize") });
        out.push(item(app, &text, "disk", "disks-menu-initialize", can_init(d), move |app| initialize(app, &dd)).danger());
    }
    out.push(MenuItem::separator());
    let id = st(&d["id"]);
    out.push(item(app, &s("properties"), "properties", "disks-menu-properties", true, move |app| app.open_disk(&id)));
    out
}
