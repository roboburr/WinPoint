use crate::{NATIVE_BRIDGE, dev_report};
use gtk::{gdk, glib, prelude::*};
use javascriptcore::ValueExt;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use webkit2gtk::{SettingsExt, UserContentManagerExt, WebContextExt, WebViewExt};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

thread_local! {
    // Windows in one app use the same local origin and trusted UI. A shared context
    // keeps one network process instead of retaining another for each modal picker.
    static WEB_CONTEXT: Rc<RefCell<wry::WebContext>> = Rc::new(RefCell::new(wry::WebContext::new(None)));
}

thread_local! {
    // Every native window with a host-owned title (contracts C4): kind and, for a picker, its options.
    static TITLED: RefCell<Vec<(glib::WeakRef<gtk::ApplicationWindow>, &'static str, Option<Value>)>> =
        const { RefCell::new(Vec::new()) };
}

/// The catalogue key (host/i18n/window.tsv) of a kind's title: the app's name as its UI shows it (C4, D-G6).
pub(crate) fn title_key(kind: &str, picker: Option<&Value>) -> &'static str {
    match kind {
        "settings" => "window.title.settings",
        "taskmgr" => "window.title.taskmgr",
        "alles" => "window.title.alles",
        "disks" => "window.title.disks",
        "defaultapps" => "window.title.defaultapps",
        "installer" => "window.title.installer",
        "picker" => {
            // the same choice as picker.rs submit() and the UI's picker title
            let options = picker.cloned().unwrap_or(Value::Null);
            let folder = options["folder"] == true;
            let save = !folder && (options["save"] == true || options["action"] == "save");
            if folder {
                "window.title.pickerFolder"
            } else if save {
                "window.title.pickerSave"
            } else {
                "window.title.pickerOpen"
            }
        }
        _ => "window.title.explorer",
    }
}

/// The native title of a window of `kind` in `lang`. A picker opened for another app through the file portal
/// (its options carry `portal: true`, portal/model.rs) keeps the title that app asked for, in that app's own
/// language. Win11's own callers (shell.pickFile from Settings, Taakbeheer, the installer, Alles) get the mode
/// title, so the title follows a language switch while the picker is open (review m3); their `title` text is
/// the page's, rendered once in the language of the moment.
pub(crate) fn title(kind: &str, picker: Option<&Value>, lang: crate::i18n::Lang) -> String {
    if kind == "picker"
        && let Some(own) = picker
            .filter(|options| options["portal"] == true)
            .and_then(|options| options["title"].as_str())
            .map(str::trim)
            .filter(|title| !title.is_empty())
    {
        return own.chars().take(200).collect();
    }
    crate::i18n::tr(lang, title_key(kind, picker))
}

/// A language switch (lib.rs watch_language): every host-titled window gets its title in `lang`.
pub(crate) fn language_changed(lang: crate::i18n::Lang) {
    let windows: Vec<_> = TITLED.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|(window, _, _)| window.upgrade().is_some());
        all.iter()
            .filter_map(|(window, kind, picker)| {
                window.upgrade().map(|w| (w, *kind, picker.clone()))
            })
            .collect()
    });
    for (window, kind, picker) in windows {
        let text = title(kind, picker.as_ref(), lang);
        window.set_title(&text);
        dev_report(
            &window,
            "title",
            &json!({"title":text,"language":lang.code()}),
        );
    }
}

/// Whether the host owns this window's title (every window `build` makes is registered: contracts C4).
fn host_titled(window: &gtk::ApplicationWindow) -> bool {
    TITLED.with(|all| {
        all.borrow()
            .iter()
            .any(|(known, _, _)| known.upgrade().is_some_and(|w| &w == window))
    })
}

/// A `setTitle` message for a window whose title the host owns: ignored, so a stray UI call cannot override C4
/// until the next switch (review m7). Logged once per process; every case is a dev report.
fn ignore_set_title(window: &gtk::ApplicationWindow, kind: &str, value: &Value) {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "Win11 {kind}: setTitle genegeerd (ignored): de host beheert de venstertitel (contracts C4)"
        );
    }
    dev_report(
        window,
        "title-ignored",
        &json!({"title": value["title"].clone()}),
    );
}

fn picker_options(address: &str) -> Option<Value> {
    url::Url::parse(address)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "picker")
        .and_then(|(_, raw)| serde_json::from_str::<Value>(&raw).ok())
        .filter(Value::is_object)
}

pub(crate) fn build(
    app: &gtk::Application,
    id: &str,
    kind: &'static str,
    address: &str,
    window_id: &str,
    requests: Rc<std::sync::mpsc::Receiver<crate::UiRequest>>,
    navigation: Rc<RefCell<Option<Value>>>,
    saved_window: Option<std::sync::Arc<crate::window_state::WindowState>>,
) {
    if let Some(window) = app.windows().first().filter(|_| kind != "picker") {
        window.present();
        return;
    }
    let (default_width, default_height, icon) = match kind {
        "settings" => (1100, 740, "preferences-system"),
        "taskmgr" => (1040, 720, "utilities-system-monitor"),
        "alles" => (1040, 680, "system-search"),
        "disks" => (1060, 760, "org.gnome.DiskUtility"),
        "defaultapps" => (1100, 740, "preferences-desktop-default-applications"),
        "picker" => (980, 680, "system-file-manager"),
        "installer" => (980, 720, "system-software-install"),
        _ => (1120, 720, "system-file-manager"),
    };
    let picker = (kind == "picker")
        .then(|| picker_options(address))
        .flatten();
    let initial_title = title(kind, picker.as_ref(), crate::i18n::current_language());
    let restored_rect = saved_window.as_ref().and_then(|state| state.normal_rect());
    let normal_width = restored_rect
        .as_ref()
        .and_then(|r| r["w"].as_i64())
        .unwrap_or(default_width as i64) as i32;
    let normal_height = restored_rect
        .as_ref()
        .and_then(|r| r["h"].as_i64())
        .unwrap_or(default_height as i64) as i32;
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(initial_title.as_str())
        .decorated(false)
        .resizable(true)
        .default_width(normal_width)
        .default_height(normal_height)
        .build();
    TITLED.with(|all| {
        let mut all = all.borrow_mut();
        // a picker is rebuilt per use: forget the closed ones
        all.retain(|(window, _, _)| window.upgrade().is_some());
        all.push((window.downgrade(), kind, picker));
    });
    window.set_role(id);
    window.set_icon_name(Some(icon));
    window.set_widget_name(window_id);
    window.set_size_request(640, 480);
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window)
        && let Some(visual) = screen.rgba_visual()
    {
        window.set_visual(Some(&visual));
    }
    window.set_app_paintable(true);
    window.connect_draw(|_, context| {
        let _ = context.save();
        context.set_operator(gtk::cairo::Operator::Source);
        context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = context.paint();
        let _ = context.restore();
        glib::Propagation::Proceed
    });
    if kind == "picker" {
        crate::picker::configure(&window, window_id);
    }
    let weak = window.downgrade();
    let picker_address = address.to_owned();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(window) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        for _ in 0..16 {
            let Ok(request) = requests.try_recv() else {
                break;
            };
            match request.kind {
                crate::UiRequestKind::FilePicker => {
                    crate::picker::show(&window, request, &picker_address)
                }
                crate::UiRequestKind::Lifecycle => {
                    let _ = request.ack.send(Ok(json!({})));
                    if let Some(application) = window.application() {
                        application.quit();
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
    let holder: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));
    let drag_region = Rc::new(RefCell::new(json!({"height":0,"noDragRects":[]})));
    let ipc_region = drag_region.clone();
    let weak_window = window.downgrade();
    let ipc_session = saved_window.clone();
    let picker_key = window_id.to_owned();
    let picker_view = Rc::downgrade(&holder);
    let native_handler: Rc<dyn Fn(Value)> = Rc::new(move |value| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        dev_report(&window, "command", &value);
        match value["cmd"].as_str().unwrap_or("") {
            "window-session" if kind == "explorer" => {
                if let Some(state) = &ipc_session {
                    state.save_session(&value["session"]);
                }
            }
            "picker-submit" if kind == "picker" => {
                // Destroying WebKit from its own IPC callback can leave the
                // detached modal alive. Validate/reply/close at the next GTK idle.
                let key = picker_key.clone();
                let weak = window.downgrade();
                let holder = picker_view.clone();
                glib::idle_add_local_once(move || {
                    let Some(window) = weak.upgrade() else { return };
                    match crate::picker::submit(&key, &value) {
                        Ok(()) => window.close(),
                        Err(error) => {
                            if let Some(holder) = holder.upgrade()
                                && let Some(view) = holder.borrow().as_ref()
                            {
                                let _ = view.evaluate_script(&format!("window.dispatchEvent(new CustomEvent('win11:picker-error',{{detail:{}}}));",json!({"message":error})));
                            }
                        }
                    }
                });
            }
            "titlebar" => *ipc_region.borrow_mut() = value,
            "minimize" => window.iconify(),
            "maximize" => window.maximize(),
            "restore" => window.unmaximize(),
            "toggleMaximize" => toggle_maximize(&window),
            "close" => {
                if kind == "picker" {
                    let weak = window.downgrade();
                    glib::idle_add_local_once(move || {
                        if let Some(window) = weak.upgrade() {
                            window.close();
                        }
                    });
                } else {
                    window.close();
                }
            }
            "setTitle" if host_titled(&window) => ignore_set_title(&window, kind, &value),
            "setTitle" => {
                if let Some(title) = value["title"].as_str() {
                    window.set_title(title);
                }
            }
            // Titlebar drags are handled below at GTK event time, preserving the Wayland input serial.
            "test-report" => dev_report(&window, "test", &value["data"]),
            _ => {}
        }
    });
    let wry_handler = native_handler.clone();
    let shared_context = WEB_CONTEXT.with(Clone::clone);
    let mut context_guard = shared_context.borrow_mut();
    let builder = if std::env::var("WIN11_PRIVATE_WEB_CONTEXT").as_deref()==Ok("1") {
        WebViewBuilder::new()
    } else { WebViewBuilder::new_with_web_context(&mut context_guard) }
        .with_url(address)
        .with_transparent(true)
        .with_initialization_script(format!(
            "{NATIVE_BRIDGE} window.__win11shellHost.windowId={};window.__win11shellHost.explorerSession={};window.__win11shellHost.nativeBackdrop={};",
            serde_json::to_string(window_id).expect("windowID"), saved_window.as_ref().map(|state|state.explorer_session()).unwrap_or(Value::Null), matches!(kind,"explorer"|"settings"|"taskmgr"|"disks"|"defaultapps"|"installer"|"alles")
        ))
        .with_ipc_handler(move |request| {
            if let Ok(value) = serde_json::from_str::<Value>(request.body()) {
                wry_handler(value);
            }
        });
    let builder = if kind == "installer" {
        let weak = Rc::downgrade(&holder);
        let drop_window = window.downgrade();
        builder.with_drag_drop_handler(move |event| {
            if let Some(window) = drop_window.upgrade() {
                dev_report(&window, "file-drag", &json!({"event":format!("{event:?}")}));
            }
            if let wry::DragDropEvent::Drop { paths, .. } = event {
                let paths: Vec<&str> = paths.iter().take(64)
                    .filter(|path| path.is_absolute())
                    .filter_map(|path| path.to_str())
                    .filter(|path| path.len() <= 8192 && !path.contains('\0'))
                    .collect();
                if let Some(holder) = weak.upgrade()
                    && let Some(view) = holder.borrow().as_ref()
                {
                    let detail = json!({"paths":paths});
                    let _ = view.evaluate_script(&format!(
                        "window.dispatchEvent(new CustomEvent('win11:files-dropped',{{detail:{detail}}}));"
                    ));
                }
            }
            true
        })
    } else {
        builder
    };
    let builder = if let Some(script) =
        std::env::var_os("WIN11_DEV_SCRIPT").and_then(|path| std::fs::read_to_string(path).ok())
    {
        builder.with_initialization_script(script)
    } else {
        builder
    };
    let origin = url::Url::parse(address).ok().map(|url| url.origin());
    let builder = builder.with_navigation_handler(move |target| {
        if url::Url::parse(&target).ok().map(|url| url.origin()) == origin {
            return true;
        }
        if target.starts_with("https://") || target.starts_with("http://") {
            let _ = crate::files::actions::open_external_uri(&target);
        }
        false
    });
    let view = match builder.build_gtk(&window) {
        Ok(view) => view,
        Err(err) => {
            eprintln!("WebKit kon niet starten: {err}");
            window.close();
            return;
        }
    };
    drop(context_guard);
    let widget = view.webview();
    if let Some(context) = WebViewExt::context(&widget) {
        // Each host keeps one local app document. Its live DOM and JS state stay
        // resident, while a browser's history/resource cache is unnecessary.
        let model = if std::env::var("WIN11_WEBKIT_BROWSER_CACHE").as_deref() == Ok("1") {
            webkit2gtk::CacheModel::WebBrowser
        } else {
            webkit2gtk::CacheModel::DocumentViewer
        };
        context.set_cache_model(model);
        dev_report(
            &window,
            "webkit-cache",
            &json!({"model":format!("{model:?}")}),
        );
    }
    let no_gpu = gtk::glib::user_config_dir()
        .join("win11-gnome/no-gpu")
        .exists()
        || gtk::glib::home_dir()
            .join(".config/win11shell/no-gpu")
            .exists();
    let gpu_mode = std::env::var("WIN11SHELL_GPU").unwrap_or_default();
    if (no_gpu || !matches!(gpu_mode.as_str(), "safe" | "env"))
        && let Some(settings) = WebViewExt::settings(&widget)
    {
        // Preserve the reference host's default software-rendering policy.
        settings.set_hardware_acceleration_policy(webkit2gtk::HardwareAccelerationPolicy::Never);
    }
    let cancel_view = Rc::downgrade(&holder);
    let drag_window = window.downgrade();
    widget.connect_drag_failed(move |_, _, result| {
        // GTK3/Wayland reports both an ordinary desktop release and Escape as Error.
        // Mutter records raw Escape order as well as the DnD-end position/button state;
        // coalesced input may already show a released button after Escape. Ambiguous
        // or unavailable evidence cancels.
        let desktop_release = kind == "explorer"
            && result == gtk::DragResult::Error
            && crate::gnome::call("explorer.tabPosition", &json!({"pid":std::process::id()}))
                .is_ok_and(|position| released_desktop_drop(&position));
        if let Some(window) = drag_window.upgrade() {
            dev_report(
                &window,
                "drag-failed",
                &json!({"result":format!("{result:?}"),"desktopRelease":desktop_release}),
            );
        }
        if result != gtk::DragResult::NoTarget
            && result != gtk::DragResult::Success
            && !desktop_release
            && let Some(holder) = cancel_view.upgrade()
            && let Some(view) = holder.borrow().as_ref()
        {
            let _ = view.evaluate_script(
                "window.dispatchEvent(new CustomEvent('win11:tab-drag-cancelled'));",
            );
        }
        glib::Propagation::Proceed
    });
    if let Some(manager) = widget.user_content_manager() {
        manager.connect_script_message_received(Some("win11"), move |_, message| {
            if let Some(value) = message.js_value().and_then(|value| value.to_json(0))
                && let Ok(value) = serde_json::from_str::<Value>(&value)
            {
                native_handler(value);
            }
        });
        if !manager.register_script_message_handler("win11") {
            eprintln!("Native vensterkanaal kon niet worden geregistreerd");
            window.close();
            return;
        }
    }

    widget.add_events(gdk::EventMask::BUTTON_PRESS_MASK | gdk::EventMask::POINTER_MOTION_MASK);
    let weak = window.downgrade();
    widget.connect_button_press_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let (x, y) = event.position();
        let (rx, ry) = event.root();
        if event.button() == 1
            && !window.is_maximized()
            && let Some(edge) =
                resize_edge(x, y, window.allocated_width(), window.allocated_height())
        {
            window.begin_resize_drag(edge, 1, rx as i32, ry as i32, event.time());
            return glib::Propagation::Stop;
        }
        let region = drag_region.borrow();
        if y < region["height"].as_f64().unwrap_or(0.0)
            && !excluded(&region, x, y)
            && event.button() == 1
        {
            if event.event_type() == gdk::EventType::DoubleButtonPress {
                toggle_maximize(&window);
            } else {
                window.begin_move_drag(1, rx as i32, ry as i32, event.time());
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    let weak = window.downgrade();
    let last_cursor = Cell::new(false);
    widget.connect_motion_notify_event(move |widget, event| {
        let Some(window) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let (x, y) = event.position();
        let edge = if !window.is_maximized() {
            resize_edge(x, y, window.allocated_width(), window.allocated_height())
        } else {
            None
        };
        if let Some(gdk_window) = widget.window() {
            if let Some(edge) = edge {
                let name = match edge {
                    gdk::WindowEdge::NorthWest | gdk::WindowEdge::SouthEast => "nwse-resize",
                    gdk::WindowEdge::NorthEast | gdk::WindowEdge::SouthWest => "nesw-resize",
                    gdk::WindowEdge::North | gdk::WindowEdge::South => "ns-resize",
                    _ => "ew-resize",
                };
                let cursor = gdk::Cursor::from_name(&gdk_window.display(), name);
                gdk_window.set_cursor(cursor.as_ref());
                last_cursor.set(true);
            } else if last_cursor.replace(false) {
                gdk_window.set_cursor(None);
            }
        }
        glib::Propagation::Proceed
    });
    *holder.borrow_mut() = Some(view);
    if kind == "alles" {
        let weak = Rc::downgrade(&holder);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(holder) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some(params) = navigation.borrow_mut().take()
                && let Some(view) = holder.borrow().as_ref()
            {
                let _ = view.evaluate_script(&format!(
                    "(async()=>{{while(document.documentElement.dataset.ready!=='true')await new Promise(r=>setTimeout(r,50));const{{bridge}}=await import(new URL('/ui/js/core/bridge.js',location.href).href);bridge.emitLocal('app.navigate',{{app:'alles',params:{params}}});}})()"
                ));
            }
            glib::ControlFlow::Continue
        });
    }
    // Explicit local development hook; no HTTP/eval endpoint exists in normal app use.
    if let Some(path) = std::env::var_os("WIN11_DEV_COMMAND") {
        let weak = Rc::downgrade(&holder);
        let window_id = window_id.to_owned();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(holder) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Ok(script) = std::fs::read_to_string(&path)
                && !script.trim().is_empty()
            {
                let script = if let Ok(value) = serde_json::from_str::<Value>(&script) {
                    if value["windowId"].as_str() != Some(window_id.as_str()) {
                        return glib::ControlFlow::Continue;
                    }
                    value["script"].as_str().unwrap_or("").to_owned()
                } else {
                    script
                };
                let _ = std::fs::write(&path, "");
                if let Some(view) = holder.borrow().as_ref() {
                    let _ = view.evaluate_script(&script);
                }
            }
            glib::ControlFlow::Continue
        });
    }
    let changed_session = saved_window.clone();
    let weak_view = Rc::downgrade(&holder);
    window.connect_window_state_event(move |window, event| {
        if let Some(state) = &changed_session { state.capture(); }
        dev_report(window, "window-state", &json!({}));
        if let Some(holder) = weak_view.upgrade() && let Some(view) = holder.borrow().as_ref() {
            let state = json!({"maximized":event.new_window_state().contains(gdk::WindowState::MAXIMIZED),
                "focused":window.is_active(),"fullscreen":event.new_window_state().contains(gdk::WindowState::FULLSCREEN),"tiled":false});
            let _ = view.evaluate_script(&format!("window.__win11WindowState={state};window.dispatchEvent(new CustomEvent('win11:windowstate',{{detail:window.__win11WindowState}}));"));
        }
        glib::Propagation::Proceed
    });
    let closing_session = saved_window.clone();
    let closed_picker_key = window_id.to_owned();
    let closing = Cell::new(false);
    window.connect_delete_event(move |window, _| {
        if kind == "picker" {
            // Pickers have no Explorer session to flush. Release their strong
            // pending reference on the first native delete, before another modal opens.
            dev_report(window, "closed", &json!({}));
            crate::picker::closed(&closed_picker_key);
            holder.borrow_mut().take();
            return glib::Propagation::Proceed;
        }
        if let Some(state) = &closing_session {
            state.capture();
        }
        if closing.replace(true) {
            dev_report(window, "closed", &json!({}));
            holder.borrow_mut().take();
            return glib::Propagation::Proceed;
        }
        if let Some(view) = holder.borrow().as_ref() {
            let _ = view.evaluate_script("window.dispatchEvent(new CustomEvent('win11:close'));");
        }
        let weak = window.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        glib::Propagation::Stop
    });
    if let Some(state) = &saved_window {
        if state.maximized() {
            window.maximize();
        }
    }
    window.show_all();
    // Wry may realize/map the GTK container while constructing WebKit. Queue after showing
    // rather than depending on a map signal that could already have been delivered.
    if let Some(state) = &saved_window {
        state.restore();
    }
}

fn toggle_maximize(window: &gtk::ApplicationWindow) {
    if window.is_maximized() {
        window.unmaximize();
    } else {
        window.maximize();
    }
}

fn released_desktop_drop(position: &Value) -> bool {
    position["ended"] == true
        && position["buttonPressed"] == false
        && position["outside"] == true
        && position["cancelled"] == false
}

fn excluded(region: &Value, x: f64, y: f64) -> bool {
    region["noDragRects"].as_array().is_some_and(|rects| {
        rects.iter().any(|r| {
            let left = r["x"].as_f64().unwrap_or(0.0);
            let top = r["y"].as_f64().unwrap_or(0.0);
            x >= left
                && y >= top
                && x < left + r["w"].as_f64().unwrap_or(0.0)
                && y < top + r["h"].as_f64().unwrap_or(0.0)
        })
    })
}

fn resize_edge(x: f64, y: f64, width: i32, height: i32) -> Option<gdk::WindowEdge> {
    match (
        x < 6.0,
        x >= f64::from(width - 6),
        y < 6.0,
        y >= f64::from(height - 6),
    ) {
        (true, _, true, _) => Some(gdk::WindowEdge::NorthWest),
        (_, true, true, _) => Some(gdk::WindowEdge::NorthEast),
        (true, _, _, true) => Some(gdk::WindowEdge::SouthWest),
        (_, true, _, true) => Some(gdk::WindowEdge::SouthEast),
        (true, _, _, _) => Some(gdk::WindowEdge::West),
        (_, true, _, _) => Some(gdk::WindowEdge::East),
        (_, _, true, _) => Some(gdk::WindowEdge::North),
        (_, _, _, true) => Some(gdk::WindowEdge::South),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_detach_requires_a_confirmed_release_outside_the_source() {
        assert!(released_desktop_drop(
            &json!({"ended":true,"buttonPressed":false,"outside":true,"cancelled":false})
        ));
        assert!(!released_desktop_drop(
            &json!({"ended":true,"buttonPressed":false,"outside":true,"cancelled":true})
        ));
        assert!(!released_desktop_drop(
            &json!({"ended":true,"buttonPressed":false,"outside":true})
        ));
        assert!(!released_desktop_drop(
            &json!({"ended":true,"buttonPressed":true,"outside":true})
        ));
        assert!(!released_desktop_drop(
            &json!({"ended":false,"buttonPressed":false,"outside":true})
        ));
        assert!(!released_desktop_drop(
            &json!({"ended":true,"buttonPressed":false,"outside":false})
        ));
        assert!(!released_desktop_drop(&json!({})));
    }
    #[test]
    fn caption_buttons_and_tabs_are_excluded_from_dragging() {
        let region =
            json!({"noDragRects":[{"x":950,"y":0,"w":138,"h":32},{"x":8,"y":4,"w":240,"h":32}]});
        assert!(excluded(&region, 980.0, 15.0));
        assert!(excluded(&region, 120.0, 20.0));
        assert!(!excluded(&region, 700.0, 20.0));
    }
    #[test]
    fn resize_edges_and_content_are_distinct() {
        assert_eq!(
            resize_edge(1.0, 1.0, 1120, 720),
            Some(gdk::WindowEdge::NorthWest)
        );
        assert_eq!(
            resize_edge(1118.0, 400.0, 1120, 720),
            Some(gdk::WindowEdge::East)
        );
        assert_eq!(resize_edge(100.0, 100.0, 1120, 720), None);
    }
}
#[cfg(test)]
mod title_tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn every_kind_has_its_ui_name_in_both_languages() {
        let cases = [
            ("settings", "Instellingen", "Settings"),
            ("taskmgr", "Taakbeheer", "Task Manager"),
            ("alles", "Alles", "Alles"),
            ("disks", "Schijven", "Disks"),
            ("defaultapps", "Standaard-apps", "Default apps"),
            ("installer", "App-installatieprogramma", "App Installer"),
            ("explorer", "Verkenner", "File Explorer"),
            ("unknown-kind", "Verkenner", "File Explorer"),
            ("picker", "Openen", "Open"),
        ];
        for (kind, nl, en) in cases {
            assert_eq!(title(kind, None, Lang::Nl), nl, "{kind}");
            assert_eq!(title(kind, None, Lang::En), en, "{kind}");
        }
    }

    #[test]
    fn picker_titles_follow_the_mode_and_a_portal_callers_own_title() {
        let save = json!({"save": true});
        let action = json!({"action": "save"});
        let folder = json!({"folder": true, "save": true});
        assert_eq!(title("picker", Some(&save), Lang::Nl), "Opslaan als");
        assert_eq!(title("picker", Some(&action), Lang::En), "Save as");
        assert_eq!(title("picker", Some(&folder), Lang::Nl), "Map kiezen");
        assert_eq!(title("picker", Some(&folder), Lang::En), "Choose a folder");
        // another app through the file portal: its own title, in its own language
        let own = json!({"portal": true, "title": "  Export Image  ", "save": true});
        assert_eq!(title("picker", Some(&own), Lang::Nl), "Export Image");
        assert_eq!(title("picker", Some(&own), Lang::En), "Export Image");
        let empty = json!({"portal": true, "title": "   "});
        assert_eq!(title("picker", Some(&empty), Lang::En), "Open");
        let long = json!({"portal": true, "title": "x".repeat(500)});
        assert_eq!(title("picker", Some(&long), Lang::En).chars().count(), 200);
        // Win11's own callers (shell.pickFile): the mode title, so a switch re-titles the open picker (review m3)
        let in_app = json!({"title": "Een foto kiezen voor je bureaublad"});
        assert_eq!(title("picker", Some(&in_app), Lang::Nl), "Openen");
        assert_eq!(title("picker", Some(&in_app), Lang::En), "Open");
        let in_app_folder =
            json!({"title": "Map kiezen voor de index", "folder": true, "portal": "yes"});
        assert_eq!(
            title("picker", Some(&in_app_folder), Lang::En),
            "Choose a folder"
        );
    }

    #[test]
    fn picker_options_come_from_the_window_address() {
        let mut url = url::Url::parse("http://127.0.0.1:1/ui/index.html").unwrap();
        url.query_pairs_mut()
            .append_pair("token", "t")
            .append_pair("picker", &json!({"folder": true}).to_string());
        assert_eq!(picker_options(url.as_str()), Some(json!({"folder": true})));
        assert_eq!(
            picker_options("http://127.0.0.1:1/ui/index.html?token=t"),
            None
        );
        assert_eq!(picker_options("http://127.0.0.1:1/?picker=notjson"), None);
    }
}
