//! The harness's dev channel (native/shell/src/dev.rs for apps): with WIN11_DEV_COMMAND (a file the harness
//! writes one JSON command to) and WIN11_DEV_REPORT (a JSON-lines file reports are appended to), the app reports
//!   {"event":"test","windowId":"native-<kind>","data":{"tag":"boot","kind":"native-app","app":kind,"pid":pid,
//!    "renderer":GSK_RENDERER,"fontRendering":"manual"|"automatic"}}
//! at start (fontRendering: GtkSettings' gtk-font-rendering as the app runs with it, app.rs sets manual under cairo)
//! and runs commands with a "native" key whose "app" is this app's kind (the shell and the other apps share
//! the command file: a command without "app" is the shell's, native/shell/src/dev.rs):
//!   {"native":"state","tag":t}                    the widget trees of all windows (name, type, css classes,
//!                                                  label/text, allocation rect, visible, sensitive); a widget
//!                                                  GTK does not show (hidden, or not child-visible: the rows of
//!                                                  a closed expander's revealer, a stack's other pages) is left
//!                                                  out with everything under it
//!   {"native":"click","name":n,"tag":t}           activate the widget with that name (button click, toggle,
//!                                                  switch, check, a clickable SettingsCard's button, or the
//!                                                  widget's activate); "click", "type" and "key" take a widget
//!                                                  of that name that is shown first
//!   {"native":"type","name":n,"text":s,"tag":t}   set the text of an entry / text / text view by name
//!   {"native":"key","name":n,"key":k,"tag":t}     a key ("Return", "Escape", "<Control>f", "Down", "Tab"):
//!                                                  the window's AppWindow shortcuts first, then Return
//!                                                  activates, Escape closes the top dialog or menu, arrows and
//!                                                  Tab move the focus from the named widget
//!   {"native":"screenshot","path":p,"tag":t}    render every visible window offscreen (GSK's cairo renderer) to
//!                                                  PNG: p, then p-1.png, ... (no compositor needed)
//! Each answer is {"event":"test","windowId":"native-<kind>","data":{"tag":t,"value":...}}.
use crate::ctx::Ctx;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;

pub(crate) fn install(ctx: &Rc<Ctx>) {
    let (Some(command), Some(report)) = (std::env::var_os("WIN11_DEV_COMMAND"), std::env::var_os("WIN11_DEV_REPORT")) else {
        return;
    };
    let kind = ctx.kind();
    let window_id = format!("native-{kind}");
    let report = std::path::PathBuf::from(report);
    let renderer = std::env::var("GSK_RENDERER").ok();
    let font_rendering = gtk::Settings::default().map(|s| match s.gtk_font_rendering() {
        gtk::FontRendering::Manual => "manual".to_owned(),
        gtk::FontRendering::Automatic => "automatic".to_owned(),
        other => format!("{other:?}"),
    });
    write(
        &report,
        json!({"event":"test","windowId":window_id,"data":{"tag":"boot","kind":"native-app","app":kind,"pid":std::process::id(),
            "renderer":renderer,"fontRendering":font_rendering}}),
    );
    let command = std::path::PathBuf::from(command);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Ok(text) = std::fs::read_to_string(&command)
            && let Ok(v) = serde_json::from_str::<Value>(&text)
            && v.get("native").is_some()
            && v.get("app").and_then(Value::as_str).is_some_and(|a| a == kind)
        {
            let _ = std::fs::write(&command, "");
            let tag = v["tag"].clone();
            let value = run(&v);
            write(&report, json!({"event":"test","windowId":window_id,"data":{"tag":tag,"value":value}}));
        }
        glib::ControlFlow::Continue
    });
}

fn write(path: &std::path::Path, line: Value) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

fn windows() -> Vec<gtk::Window> {
    gtk::Window::list_toplevels().into_iter().filter_map(|w| w.downcast::<gtk::Window>().ok()).collect()
}

/// The widget called `name` in any window, a visible one first: two widgets can share a name (the frame's hidden
/// title-bar Back "<kind>-back" and Explorer's own "explorer-back" in its address row), and the one on screen is
/// the one meant.
fn find_named(name: &str) -> Option<gtk::Widget> {
    for visible in [true, false] {
        for w in windows() {
            if let Some(found) = find_in(w.upcast_ref(), name, visible) {
                return Some(found);
            }
        }
    }
    None
}

/// Run one command (also usable from tests inside the app).
pub fn run(v: &Value) -> Value {
    match v["native"].as_str().unwrap_or("") {
        "state" => json!(windows()
            .into_iter()
            .map(|w| json!({"title": w.title().map(|t| t.to_string()), "visible": w.is_visible(), "active": w.is_active(),
                "width": w.width(), "height": w.height(), "tree": tree(w.upcast_ref())}))
            .collect::<Vec<_>>()),
        "click" => {
            let name = v["name"].as_str().unwrap_or("");
            match find_named(name) {
                Some(widget) => {
                    activate(&widget);
                    json!({ "clicked": name })
                }
                None => json!({ "error": format!("no widget {name}") }),
            }
        }
        "type" => {
            let name = v["name"].as_str().unwrap_or("");
            let text = v["text"].as_str().unwrap_or("");
            match find_named(name) {
                Some(widget) => {
                    if let Some(e) = editable(&widget) {
                        e.set_text(text);
                        e.grab_focus();
                        json!({ "typed": name })
                    } else if let Some(tv) = widget.downcast_ref::<gtk::TextView>() {
                        tv.buffer().set_text(text);
                        json!({ "typed": name })
                    } else {
                        json!({ "error": format!("{name} takes no text") })
                    }
                }
                None => json!({ "error": format!("no widget {name}") }),
            }
        }
        "key" => {
            let name = v["name"].as_str().unwrap_or("");
            let key = v["key"].as_str().unwrap_or("");
            let widget = if name.is_empty() { None } else { find_named(name) };
            json!({ "key": key, "handled": press(widget.as_ref(), key) })
        }
        "screenshot" => {
            let path = v["path"].as_str().unwrap_or("/tmp/win11-native.png");
            let base = path.trim_end_matches(".png").to_owned();
            let mut files = Vec::new();
            let mut errors = Vec::new();
            for (i, w) in windows().into_iter().filter(|w| w.is_visible()).enumerate() {
                let p = if i == 0 { path.to_owned() } else { format!("{base}-{i}.png") };
                match screenshot(&w, &p) {
                    Ok(()) => files.push(p),
                    Err(e) => errors.push(e),
                }
            }
            json!({ "files": files, "errors": errors })
        }
        other => json!({ "error": format!("unknown command {other}") }),
    }
}

/// Render a window offscreen to a PNG (also without a compositor).
pub fn screenshot(window: &gtk::Window, path: &str) -> Result<(), String> {
    let (w, h) = (window.width(), window.height());
    if w <= 0 || h <= 0 {
        return Err("window not allocated".into());
    }
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, w as f64, h as f64);
    let node = snap.to_node().ok_or("nothing drawn")?;
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize_for_display(&WidgetExt::display(window)).map_err(|e| e.to_string())?;
    let texture = renderer.render_texture(&node, None);
    renderer.unrealize();
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn editable(w: &gtk::Widget) -> Option<gtk::Editable> {
    if let Some(e) = w.dynamic_cast_ref::<gtk::Editable>() {
        return Some(e.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(e) = editable(&child) {
            return Some(e);
        }
        c = child.next_sibling();
    }
    None
}

fn activate(widget: &gtk::Widget) {
    if let Some(b) = widget.downcast_ref::<gtk::CheckButton>() {
        // a radio button only turns on; a check box toggles
        b.set_active(b.has_css_class("w11-radio") || !b.is_active());
    } else if let Some(b) = widget.downcast_ref::<gtk::ToggleButton>() {
        b.set_active(!b.is_active());
    } else if let Some(b) = widget.downcast_ref::<gtk::Button>() {
        b.emit_clicked();
    } else if let Some(s) = widget.downcast_ref::<gtk::Switch>() {
        s.set_active(!s.is_active());
    } else if let Some(b) = widget.downcast_ref::<gtk::MenuButton>() {
        b.popup();
    } else if let Some(o) = widget.downcast_ref::<gtk::Overlay>()
        && o.has_css_class(crate::controls::cards::CARD_HOST_CLASS)
        && let Some(button) = o.child()
    {
        // a clickable SettingsCard named through `card.widget()`: its button (the control lies beside it)
        activate(&button);
    } else {
        widget.activate();
    }
}

/// Whether GTK shows `widget` as far as its parent goes: visible, and child-visible (a GtkRevealer that is closed
/// and a GtkStack's other pages keep their children visible but not child-visible).
fn shown(widget: &gtk::Widget) -> bool {
    widget.is_visible() && widget.is_child_visible()
}

fn press(widget: Option<&gtk::Widget>, key: &str) -> bool {
    let window = widget
        .and_then(|w| crate::frame::AppWindow::of(w))
        .or_else(crate::frame::AppWindow::active);
    if let Some(w) = &window
        && !w.dialog_open()
        && w.run_shortcut(key)
    {
        return true;
    }
    let Some((k, _mods)) = gtk::accelerator_parse(key) else { return false };
    match k {
        gdk::Key::Return | gdk::Key::KP_Enter => {
            if let Some(w) = widget {
                if let Some(e) = w.downcast_ref::<gtk::Entry>() {
                    e.emit_activate();
                } else if let Some(t) = w.downcast_ref::<gtk::Text>() {
                    t.emit_by_name::<()>("activate", &[]);
                } else {
                    activate(w);
                }
                return true;
            }
            false
        }
        gdk::Key::Escape => {
            crate::controls::menu::close_all();
            if let Some(w) = &window
                && let Some(layer) = w.top_dialog()
                && let Some(b) = find(&layer, "w11-dialog-close")
            {
                activate(&b);
            }
            true
        }
        gdk::Key::Down | gdk::Key::Up | gdk::Key::Left | gdk::Key::Right | gdk::Key::Tab => {
            let dir = match k {
                gdk::Key::Down => gtk::DirectionType::Down,
                gdk::Key::Up => gtk::DirectionType::Up,
                gdk::Key::Left => gtk::DirectionType::Left,
                gdk::Key::Right => gtk::DirectionType::Right,
                _ => gtk::DirectionType::TabForward,
            };
            match widget {
                Some(w) => {
                    w.grab_focus();
                    w.root().is_some_and(|r| r.upcast_ref::<gtk::Widget>().child_focus(dir))
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// The widget called `name` under `widget`, a visible one first.
fn find(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    find_in(widget, name, true).or_else(|| find_in(widget, name, false))
}

/// Depth first; with `visible`, only through widgets that are shown themselves (hidden subtrees and the content of
/// a closed expander are skipped; the second pass without `visible` still finds them by name).
fn find_in(widget: &gtk::Widget, name: &str, visible: bool) -> Option<gtk::Widget> {
    if visible && !shown(widget) {
        return None;
    }
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_in(&c, name, visible) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn tree(widget: &gtk::Widget) -> Value {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(c) = child {
        if shown(&c) {
            children.push(tree(&c));
        }
        child = c.next_sibling();
    }
    let label = widget
        .downcast_ref::<gtk::Label>()
        .map(|l| l.label().to_string())
        .or_else(|| widget.downcast_ref::<gtk::Button>().and_then(|b| b.label()).map(|s| s.to_string()))
        .or_else(|| widget.dynamic_cast_ref::<gtk::Editable>().map(|e| e.text().to_string()))
        .or_else(|| widget.tooltip_text().map(|t| t.to_string()));
    let bounds = widget.root().and_then(|r| widget.compute_bounds(&r));
    let mut v = json!({"type": widget.type_().name(), "name": widget.widget_name().to_string(),
        "classes": widget.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "label": label,
        "visible": shown(widget),
        "sensitive": widget.is_sensitive(),
        "rect": bounds.map(|b| [b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32]),
        "children": children});
    if let Some(s) = widget.downcast_ref::<gtk::Switch>() {
        v["active"] = json!(s.is_active());
    } else if let Some(c) = widget.downcast_ref::<gtk::CheckButton>() {
        v["active"] = json!(c.is_active());
    }
    v
}

#[cfg(test)]
#[path = "dev_tests.rs"]
mod tests;
