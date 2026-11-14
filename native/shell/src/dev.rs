//! The harness's dev channel for native surfaces (tests/gnome): WIN11_DEV_COMMAND is a file the harness writes
//! one JSON command to, WIN11_DEV_REPORT a JSON-lines file the shell appends to. Only commands with a "native"
//! key and no "app" key are ours (the native apps, native/kit dev.rs, share the file); each report is
//! {"event":"test","data":{"tag",...}}.
//!   {"native":"state","tag":t}             the surfaces' widget trees (name, css classes, label, allocation; an
//!                                           image's icon: its theme name or file, "paintable" for a fetched one)
//!   {"native":"click","name":n,"tag":t}     activate the widget with this name (gtk::Widget::set_widget_name)
//!   {"native":"type","name":n,"text":s,"tag":t}  set the text of the editable with this name (Start's
//!                                           "start-search")
//!   {"native":"flyout","name":n,"tag":t}    toggle a flyout
use crate::core::Shell;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;

pub fn install(_shell: &Rc<Shell>) {
    let (Some(command), Some(report)) = (std::env::var_os("WIN11_DEV_COMMAND"), std::env::var_os("WIN11_DEV_REPORT"))
    else {
        return;
    };
    let report = std::path::PathBuf::from(report);
    write(&report, json!({"event":"test","windowId":"native","data":{"tag":"boot","kind":"native-shell","pid":std::process::id()}}));
    let command = std::path::PathBuf::from(command);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Ok(text) = std::fs::read_to_string(&command)
            && let Ok(v) = serde_json::from_str::<Value>(&text)
            && v.get("native").is_some()
            && v.get("app").is_none()
        {
            let _ = std::fs::write(&command, "");
            let tag = v["tag"].clone();
            let value = run(&v);
            write(&report, json!({"event":"test","windowId":"native","data":{"tag":tag,"value":value}}));
        }
        glib::ControlFlow::Continue
    });
}

fn write(path: &std::path::Path, line: Value) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

fn run(v: &Value) -> Value {
    match v["native"].as_str().unwrap_or("") {
        "state" => json!(gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .map(|w| json!({"title": w.title().map(|t| t.to_string()), "visible": w.is_visible(),
                "width": w.width(), "height": w.height(), "tree": tree(w.upcast_ref())}))
            .collect::<Vec<_>>()),
        "click" => {
            let name = v["name"].as_str().unwrap_or("");
            let found = gtk::Window::list_toplevels()
                .into_iter()
                .find_map(|w| find(&w, name));
            match found {
                Some(widget) => {
                    if let Some(b) = widget.downcast_ref::<gtk::Button>() {
                        b.emit_clicked();
                    } else {
                        widget.activate();
                    }
                    json!({"clicked": name})
                }
                None => json!({"error": format!("no widget {name}")}),
            }
        }
        "type" => {
            let name = v["name"].as_str().unwrap_or("");
            let found = gtk::Window::list_toplevels().into_iter().find_map(|w| find(&w, name));
            match found.as_ref().and_then(|w| w.dynamic_cast_ref::<gtk::Editable>()) {
                Some(editable) => {
                    editable.set_text(v["text"].as_str().unwrap_or(""));
                    json!({"typed": name})
                }
                None if found.is_some() => json!({"error": format!("{name} takes no text")}),
                None => json!({"error": format!("no widget {name}")}),
            }
        }
        "flyout" => {
            let shell = crate::core::shell();
            let name: &'static str = Box::leak(v["name"].as_str().unwrap_or("").to_owned().into_boxed_str());
            shell.flyouts().toggle(name);
            json!({"open": shell.flyouts().current()})
        }
        other => json!({"error": format!("unknown command {other}")}),
    }
}

fn find(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find(&c, name) {
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
        if c.is_visible() {
            children.push(tree(&c));
        }
        child = c.next_sibling();
    }
    let label = widget
        .downcast_ref::<gtk::Label>()
        .map(|l| l.label().to_string())
        .or_else(|| widget.tooltip_text().map(|t| t.to_string()));
    let bounds = widget.root().and_then(|r| widget.compute_bounds(&r));
    let icon = widget.downcast_ref::<gtk::Image>().and_then(image_icon);
    let mut node = json!({"type": widget.type_().name(), "name": widget.widget_name().to_string(),
        "classes": widget.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "label": label,
        "rect": bounds.map(|b| [b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32]),
        "children": children});
    if let Some(icon) = icon {
        node["icon"] = json!(icon);
    }
    node
}

/// What an image shows, for the tree: the icon the taskbar asked for (panel/taskbar/util.rs `set_icon` keeps
/// "<name>|<px>" on the image as "w11-icon"; it shows the backend's /icon answer for that name), else its theme
/// icon name or file, "paintable" for another picture.
fn image_icon(image: &gtk::Image) -> Option<String> {
    // SAFETY: "w11-icon" is only ever set to a String (util.rs set_icon)
    if let Some(key) = unsafe { image.data::<String>("w11-icon") } {
        let key = unsafe { key.as_ref() };
        return Some(key.rsplit_once('|').map_or(key.as_str(), |(name, _)| name).to_owned());
    }
    match image.storage_type() {
        gtk::ImageType::IconName => image.icon_name().map(|n| n.to_string()),
        gtk::ImageType::Paintable => Some(image.file().map(|f| f.to_string()).unwrap_or_else(|| "paintable".into())),
        gtk::ImageType::Gicon => image.gicon().and_then(|g| gtk::gio::prelude::IconExt::to_string(&g)).map(|s| s.to_string()),
        _ => None,
    }
}
