//! The tray's input indicator (notes/claude/settings-parity.md amendment 15, batch A's patch): the current input
//! source's short name (GNOME Shell's, in capitals, as Windows' "ENG") while more than one is set; a click lists
//! them and switches (input.sources.current / input.sources.activate through inputBridge.js); it follows
//! input.sourceChanged and input.sourcesChanged.
use super::util;
use crate::core::Shell;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Input {
    shell: Rc<Shell>,
    pub button: gtk::Button,
    label: gtk::Label,
    state: RefCell<Value>,
}

pub fn build(shell: &Rc<Shell>, strip: &gtk::Widget) -> Rc<Input> {
    let button = gtk::Button::new();
    button.add_css_class("w11-tb-hit");
    button.add_css_class("w11-tb-input");
    button.set_widget_name("tb-input");
    let label = gtk::Label::new(None);
    button.set_child(Some(&label));
    button.set_visible(false);
    let input = Rc::new(Input { shell: shell.clone(), button, label, state: RefCell::new(Value::Null) });
    let (weak, strip) = (Rc::downgrade(&input), strip.clone());
    input.button.connect_clicked(move |b| {
        if let Some(i) = weak.upgrade() {
            i.menu(&strip, util::center_x(b, &strip));
        }
    });
    for event in ["input.sourceChanged", "input.sourcesChanged"] {
        let weak = Rc::downgrade(&input);
        shell.on(event, move |_, _| {
            if let Some(i) = weak.upgrade() {
                i.reload();
            }
        });
    }
    let weak = Rc::downgrade(&input);
    shell.on("settings.changed", move |_, _| {
        if let Some(i) = weak.upgrade() { i.render(); }
    });
    input.reload();
    input
}

impl Input {
    fn reload(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.shell.call("input.sources.current", json!({}), move |r| {
            if let Some(i) = weak.upgrade() {
                *i.state.borrow_mut() = r.unwrap_or(Value::Null);
                i.render();
            }
        });
    }

    fn sources(&self) -> Vec<Value> {
        self.state.borrow()["sources"].as_array().cloned().unwrap_or_default()
    }

    fn render(&self) {
        let sources = self.sources();
        let current = self.state.borrow()["current"].as_i64().unwrap_or(-1);
        let now = sources.iter().find(|s| s["index"].as_i64() == Some(current));
        self.button.set_visible(sources.len() > 1);
        self.label.set_text(&now.and_then(|s| s["short"].as_str()).unwrap_or("").to_uppercase());
        let name = now.and_then(|s| s["name"].as_str()).unwrap_or("");
        let hint = self.shell.t("Wisselen met Windows-toets + spatiebalk", "Switch with Windows key + Space");
        self.button.set_tooltip_text(Some(&format!("{name}\n{hint}")));
    }

    fn menu(&self, strip: &gtk::Widget, x: i32) {
        let current = self.state.borrow()["current"].as_i64().unwrap_or(-1);
        let items = self
            .sources()
            .into_iter()
            .map(|s| {
                let index = s["index"].as_i64().unwrap_or(-1);
                let mark = if index == current { "✓  " } else { "    " };
                let label = format!("{mark}{}", s["name"].as_str().unwrap_or(""));
                let shell = self.shell.clone();
                util::item(label, move || shell.fire("input.sources.activate", json!({ "index": index })))
            })
            .collect();
        util::popup_menu(strip, x, items);
    }
}
