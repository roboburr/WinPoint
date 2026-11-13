//! The harness's dev channel for native surfaces (tests/gnome): WIN11_DEV_COMMAND is a file the harness writes
//! one JSON command to, WIN11_DEV_REPORT a JSON-lines file the shell appends to. Only commands with a "native"
//! key are ours (the WebKit hosts share the file); each report is {"event":"test","data":{"tag",...}}.
//!   {"native":"state","tag":t}             the surfaces' widget trees (name, css classes, label, allocation)
//!   {"native":"click","name":n,"tag":t}     activate the widget with this name (gtk::Widget::set_widget_name)
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
    json!({"type": widget.type_().name(), "name": widget.widget_name().to_string(),
        "classes": widget.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "label": label,
        "rect": bounds.map(|b| [b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32]),
        "children": children})
}
