//! Modal Explorer selection, shared by shell.pickFile and the native FileChooser portal.
use crate::{UiRequest, window};
use glib::translate::ToGlibPtr;
use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::mpsc::Sender,
};
type Reply = Sender<Result<Value, String>>;
type Configure = Rc<dyn Fn(&gtk::ApplicationWindow)>;
struct Pending {
    reply: RefCell<Option<Reply>>,
    parent: glib::WeakRef<gtk::ApplicationWindow>,
    parent_destroy: RefCell<Option<glib::SignalHandlerId>>,
    window: RefCell<Option<gtk::ApplicationWindow>>,
    application: gtk::Application,
    options: Value,
    rich: bool,
    configure: Option<Configure>,
}
thread_local! {
 static PENDING:RefCell<HashMap<String,Rc<Pending>>>=RefCell::new(HashMap::new());
 static SEQUENCE:Cell<u64>=const{Cell::new(0)};
}
pub(crate) fn show(parent: &gtk::ApplicationWindow, request: UiRequest, address: &str) {
    let ack = request.ack.clone();
    if let Err(error) = create(parent, request.payload, address, request.ack, false, None) {
        let _ = ack.send(Err(error));
    }
}
pub(crate) fn show_portal(
    parent: &gtk::ApplicationWindow,
    options: Value,
    address: &str,
    ack: Reply,
) -> Result<String, String> {
    create(parent, options, address, ack, true, None)
}
pub(crate) fn show_portal_with_parent(
    parent: &gtk::ApplicationWindow,
    options: Value,
    address: &str,
    ack: Reply,
    configure: Configure,
) -> Result<String, String> {
    create(parent, options, address, ack, true, Some(configure))
}
fn create(
    parent: &gtk::ApplicationWindow,
    mut options: Value,
    address: &str,
    ack: Reply,
    rich: bool,
    configure: Option<Configure>,
) -> Result<String, String> {
    if !options.is_object() {
        options = json!({});
    }
    let lang = crate::i18n::current_language();
    if PENDING.with(|all| all.borrow().len()) >= 4 {
        return Err(crate::i18n::tr(lang, "picker.tooMany").into());
    }
    let application = parent
        .application()
        .ok_or_else(|| crate::i18n::tr(lang, "picker.parentClosed"))?;
    let mut url = url::Url::parse(address).map_err(|_| crate::i18n::tr(lang, "picker.invalidAddress"))?;
    let number = SEQUENCE.with(|n| {
        let v = n.get().wrapping_add(1);
        n.set(v);
        v
    });
    let id = format!(
        "io.github.win11shell.FilePicker.p{}.n{number}",
        std::process::id()
    );
    let key = format!("picker-{}-{number}", std::process::id());
    let token = url
        .query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, v)| v.into_owned());
    let directory = options["directory"]
        .as_str()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .unwrap_or_else(glib::home_dir);
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        if let Some(token) = token {
            query.append_pair("token", &token);
        }
        query.extend_pairs([("surface", "app"), ("app", "explorer")]);
        query.append_pair("picker", &options.to_string());
        query.append_pair("path", &directory.to_string_lossy());
    }
    let pending = Rc::new(Pending {
        reply: RefCell::new(Some(ack)),
        parent: parent.downgrade(),
        parent_destroy: RefCell::new(None),
        window: RefCell::new(None),
        application: application.clone(),
        options,
        rich,
        configure,
    });
    PENDING.with(|all| all.borrow_mut().insert(key.clone(), pending.clone()));
    let child_key = key.clone();
    *pending.parent_destroy.borrow_mut() =
        Some(parent.connect_destroy(move |_| cancel(&child_key)));
    let (_, receiver) = std::sync::mpsc::channel();
    window::build(
        &application,
        &id,
        "picker",
        url.as_str(),
        &key,
        Rc::new(receiver),
        Rc::new(RefCell::new(None)),
        None,
    );
    if pending.window.borrow().is_none() {
        remove(&key);
        return Err(crate::i18n::tr(lang, "picker.couldNotOpen").into());
    }
    Ok(key)
}
pub(crate) fn window(key: &str) -> Option<gtk::ApplicationWindow> {
    PENDING.with(|all| {
        all.borrow()
            .get(key)
            .and_then(|p| p.window.borrow().clone())
    })
}
pub(crate) fn cancel(key: &str) {
    let pending = PENDING.with(|all| all.borrow().get(key).cloned());
    if let Some(pending) = pending {
        reply(&pending, None);
        let window = pending.window.borrow().clone();
        if let Some(window) = window {
            window.close();
        }
    }
}
pub(crate) fn configure(window: &gtk::ApplicationWindow, key: &str) {
    let pending = PENDING.with(|all| all.borrow().get(key).cloned());
    let Some(pending) = pending else { return };
    *pending.window.borrow_mut() = Some(window.clone());
    // GtkApplication supports one primary registration per process. Retain this detached
    // modal window ourselves and supply its own native identity.
    window.set_application(None::<&gtk::Application>);
    unsafe {
        gtk::ffi::gtk_window_set_wmclass(
            window.as_ptr().cast(),
            c"io.github.win11shell.FilePicker".as_ptr(),
            c"io.github.win11shell.FilePicker".as_ptr(),
        );
    }
    if let Some(parent) = pending.parent.upgrade() {
        if let Some(screen) = gtk::prelude::WidgetExt::screen(&parent) {
            window.set_screen(&screen);
            if let Some(visual) = screen.rgba_visual() {
                window.set_visual(Some(&visual));
            }
        }
        window.set_transient_for(Some(&parent));
    }
    // GTK3 creates the Wayland xdg_toplevel only when the window is mapped (gdk_wayland_window_map) and gives it
    // g_get_prgname(), the caller's app id (io.github.win11shell.Settings …). Before that,
    // gdk_wayland_window_set_application_id does nothing, so the former realize hook never took effect (GE1b
    // o2). The map signal's handlers run after GtkWindow's own map, when the toplevel exists, before its first
    // buffer is committed.
    // The picker also keeps its caller's GTK application id (gtk_shell1 D-Bus properties; a GtkApplicationWindow
    // sends them itself, but this one is detached from its GtkApplication above). That is the identity
    // frostedBridge.js windowIdentity() expects ("A transient Picker retains its parent's GTK application ID but
    // has its own Wayland protocol role"): the extension's windows.list reads get_gtk_application_id() first, so
    // the Win11 taskbar keeps a non-modal picker on its app's button, while the FilePicker role keeps it out of the
    // app's own window effects. Without it the taskbar resolved the FilePicker role through GNOME's
    // WindowTracker and drew a running "GNOME-instellingen" button for a Settings picker (review m6).
    let application_id = pending
        .application
        .application_id()
        .and_then(|id| std::ffi::CString::new(id.as_str()).ok());
    window.connect_map(move |window| {
        if let Some(native) = window.window()
            && native.display().type_().name() == "GdkWaylandDisplay"
        {
            unsafe {
                gdk_wayland_window_set_application_id(
                    native.to_glib_none().0,
                    c"io.github.win11shell.FilePicker".as_ptr(),
                );
                if let Some(application_id) = &application_id {
                    gdk_wayland_window_set_dbus_properties_libgtk_only(
                        native.to_glib_none().0,
                        application_id.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                    );
                }
            }
        }
    });
    window.set_modal(pending.options["modal"] != false);
    window.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
    window.set_skip_taskbar_hint(true);
    if let Some(configure) = &pending.configure {
        configure(window);
    }
    let key = key.to_owned();
    window.connect_destroy(move |_| remove(&key));
}
#[link(name = "gdk-3")]
unsafe extern "C" {
    fn gdk_wayland_window_set_application_id(
        window: *mut gtk::gdk::ffi::GdkWindow,
        application_id: *const std::ffi::c_char,
    );
    // gdk/wayland/gdkwaylandwindow.h (what GtkApplication calls for its windows); every string is copied, NULL = none.
    fn gdk_wayland_window_set_dbus_properties_libgtk_only(
        window: *mut gtk::gdk::ffi::GdkWindow,
        application_id: *const std::ffi::c_char,
        app_menu_path: *const std::ffi::c_char,
        menubar_path: *const std::ffi::c_char,
        window_object_path: *const std::ffi::c_char,
        application_object_path: *const std::ffi::c_char,
        unique_bus_name: *const std::ffi::c_char,
    );
}
fn reply(pending: &Pending, value: Option<Value>) {
    if let Some(sender) = pending.reply.borrow_mut().take() {
        let result = if pending.rich {
            value.unwrap_or(json!({"path":null,"paths":[],"cancelled":true}))
        } else {
            json!({"path":value.as_ref().map(|v|v["path"].clone()).unwrap_or(Value::Null)})
        };
        let _ = sender.send(Ok(result));
    }
}
fn remove(key: &str) {
    // GTK destruction can synchronously reenter us, so drop outside the map borrow.
    let pending = PENDING.with(|all| all.borrow_mut().remove(key));
    if let Some(pending) = pending {
        reply(&pending, None);
        if let Some(signal) = pending.parent_destroy.borrow_mut().take()
            && let Some(parent) = pending.parent.upgrade()
        {
            parent.disconnect(signal);
        }
    }
}
pub(crate) fn closed(key: &str) {
    // A detached GTK toplevel can remain referenced until our pending entry is
    // released. Finish its bookkeeping on the final native close as well.
    remove(key);
}
pub(crate) fn submit(key: &str, value: &Value) -> Result<(), String> {
    let lang = crate::i18n::current_language();
    let pending = PENDING
        .with(|all| all.borrow().get(key).cloned())
        .ok_or_else(|| crate::i18n::tr(lang, "picker.closed"))?;
    let value = selection(value, &pending.options, lang)?;
    reply(&pending, Some(value));
    Ok(())
}
fn selection(value: &Value, options: &Value, lang: crate::i18n::Lang) -> Result<Value, String> {
    let folder = options["folder"] == true;
    let save = !folder && (options["save"] == true || options["action"] == "save");
    let multiple = options["multiple"] == true && !save;
    let selected = if let Some(paths) = value["paths"].as_array() {
        paths.clone()
    } else {
        vec![value.get("path").unwrap_or(value).clone()]
    };
    if selected.is_empty() || selected.len() > 1000 || (!multiple && selected.len() != 1) {
        return Err(crate::i18n::tr(lang, "picker.chooseValidSelection").into());
    }
    let mut paths = Vec::new();
    for path in selected {
        let path = path
            .as_str()
            .filter(|p| !p.is_empty() && p.len() <= 8192 && !p.contains('\0'))
            .ok_or_else(|| crate::i18n::tr(lang, "picker.chooseValidPath"))?;
        validate(Path::new(path), folder, save, lang)?;
        if !paths.iter().any(|v| v == path) {
            paths.push(path.to_owned());
        }
    }
    let filters = options["filters"].as_array().map(Vec::len).unwrap_or(0);
    let index = value["filterIndex"]
        .as_u64()
        .unwrap_or_else(|| options["currentFilter"].as_u64().unwrap_or(0));
    if index >= filters.max(1) as u64 {
        return Err(crate::i18n::tr(lang, "picker.invalidFileType").into());
    }
    let supplied = value["choices"].as_array().cloned().unwrap_or_default();
    let mut choices = Vec::new();
    for choice in options["choices"].as_array().into_iter().flatten() {
        let Some(id) = choice["id"].as_str() else {
            continue;
        };
        let selected = supplied
            .iter()
            .find(|v| v[0].as_str() == Some(id))
            .map(|v| &v[1])
            .unwrap_or(&choice["selected"]);
        let selected = selected.as_str().ok_or_else(|| crate::i18n::tr(lang, "picker.invalidOption"))?;
        let items = choice["options"].as_array().cloned().unwrap_or_default();
        if if items.is_empty() {
            !matches!(selected, "true" | "false")
        } else {
            !items.iter().any(|v| v["id"].as_str() == Some(selected))
        } {
            return Err(crate::i18n::tr(lang, "picker.invalidOption").into());
        }
        choices.push(json!([id, selected]));
    }
    let writable = if options["allowReadOnly"] == true {
        value["writable"].as_bool().unwrap_or(true)
    } else {
        options["writable"] != false
    };
    Ok(
        json!({"path":paths[0],"paths":paths,"filterIndex":index,"choices":choices,"writable":writable}),
    )
}
fn validate(path: &Path, folder: bool, save: bool, lang: crate::i18n::Lang) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(crate::i18n::tr(lang, "picker.chooseAbsolutePath").into());
    }
    if folder {
        if !path.is_dir() {
            return Err(crate::i18n::tr(lang, "picker.chooseExistingFolder").into());
        }
    } else if save {
        if path.is_dir() || !path.parent().is_some_and(Path::is_dir) {
            return Err(crate::i18n::tr(lang, "picker.chooseFileInFolder").into());
        }
    } else if !path.is_file() {
        return Err(crate::i18n::tr(lang, "picker.chooseExistingFile").into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    fn root() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("picker-selection-{}", crate::events::nonce()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn multiple_selections_are_validated_and_deduplicated() {
        let p = root();
        let a = p.join("a");
        let b = p.join("b");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        assert!(selection(&json!({"paths":[a,b]}), &json!({}), Lang::Nl).is_err());
        let v = selection(&json!({"paths":[a,a,b]}), &json!({"multiple":true}), Lang::Nl).unwrap();
        assert_eq!(v["paths"], json!([a, b]));
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn saved_selection_never_creates_a_file_and_validates_options() {
        let p = root();
        let a = p.join("new document.txt");
        let o = json!({"save":true,"filters":[{"patterns":["*.txt"]}],"choices":[{"id":"encoding","options":[{"id":"utf8","label":"UTF-8"}],"selected":"utf8"}],"allowReadOnly":true});
        let v = selection(&json!({"path":a,"filterIndex":0,"choices":[["encoding","utf8"]],"writable":false}), &o, Lang::Nl)
        .unwrap();
        assert_eq!(v["writable"], false);
        assert!(!a.exists());
        assert!(selection(&json!({"path":a,"filterIndex":1}), &o, Lang::Nl).is_err());
        assert!(selection(&json!({"path":a,"choices":[["encoding","bad"]]}), &o, Lang::Nl).is_err());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn folders_and_invalid_paths_remain_separate() {
        let p = root();
        let a = p.join("a");
        std::fs::write(&a, "").unwrap();
        assert!(selection(&json!({"path":p}), &json!({"folder":true}), Lang::Nl).is_ok());
        assert!(selection(&json!({"path":a}), &json!({"folder":true}), Lang::Nl).is_err());
        assert!(selection(&json!({"path":"relative"}), &json!({"save":true}), Lang::Nl).is_err());
        assert!(selection(&json!({"path":"/tmp/a\0b"}), &json!({"save":true}), Lang::Nl).is_err());
        std::fs::remove_dir_all(p).unwrap();
    }
    /// The en render test of the `picker` stem (round2-gate §6): the selection checks of a submitted picker.
    #[test]
    fn selection_errors_follow_the_settings_language() {
        let p = root();
        let file = p.join("a");
        std::fs::write(&file, "").unwrap();
        let error = |value: Value, options: Value, lang: Lang| selection(&value, &options, lang).unwrap_err();
        for (lang, texts) in [
            (Lang::Nl, ["Kies een geldige bestandsselectie.", "Kies een absoluut bestandspad.",
                        "Kies een bestaande map.", "Kies een bestaand bestand.", "Ongeldig bestandstype."]),
            (Lang::En, ["Choose a valid file selection.", "Choose an absolute file path.",
                        "Choose an existing folder.", "Choose an existing file.", "Invalid file type."]),
        ] {
            assert_eq!(error(json!({"paths":[]}), json!({}), lang), texts[0]);
            assert_eq!(error(json!({"path":"relative"}), json!({}), lang), texts[1]);
            assert_eq!(error(json!({"path":file}), json!({"folder":true}), lang), texts[2]);
            assert_eq!(error(json!({"path":p}), json!({}), lang), texts[3]);
            assert_eq!(error(json!({"path":file,"filterIndex":3}), json!({}), lang), texts[4]);
        }
        std::fs::remove_dir_all(p).unwrap();
    }
}
