//! The separate Windows-style file-operation window. It observes host progress; it never copies files itself.
//! Copy/move caption-close, Escape and Cancel request the same host cancellation. Minimise keeps work running.
use crate::i18n::{f, n, s};
use crate::util;
use gtk::prelude::*;
use gtk::{glib, pango};
use kit::frame::{AppWindow, WindowOptions};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

/// Byte counters remain authoritative. Rates use monotonic samples, never a wall clock or invented progress.
#[derive(Default)]
struct Metrics {
    done: u64,
    total: Option<u64>,
    /// Exact recursive counts; legacy items/count describe only the originally selected roots.
    items: Option<u64>,
    count: Option<u64>,
    current: String,
    state: String,
    can_pause: bool,
    can_cancel: bool,
    samples: VecDeque<(f64, u64)>,
    graph: VecDeque<f64>,
}

#[derive(Clone, Copy)]
pub(crate) struct ControlStamp {
    request: u64,
    progress: u64,
}

/// SSE and HTTP have independent delivery order. An old response must never roll back newer host progress.
#[derive(Default)]
struct ControlOrder {
    request: u64,
    progress: u64,
}

impl ControlOrder {
    fn begin(&mut self) -> ControlStamp {
        self.request = self.request.wrapping_add(1);
        ControlStamp {
            request: self.request,
            progress: self.progress,
        }
    }

    fn note_progress(&mut self) {
        self.progress = self.progress.wrapping_add(1);
    }

    fn current(&self, stamp: ControlStamp) -> bool {
        self.request == stamp.request
    }

    fn may_apply(&self, stamp: ControlStamp) -> bool {
        self.current(stamp) && self.progress == stamp.progress
    }
}

impl Metrics {
    fn observe(&mut self, value: &Value, now: f64) {
        let next_state = value["state"].as_str().unwrap_or("running");
        let paused = self.state == "paused" || next_state == "paused";
        let done = value["done"].as_u64().unwrap_or(self.done);
        if paused || done < self.done {
            self.samples.clear();
        }
        self.done = done;
        self.total = value["total"].as_u64();
        self.items = value["entriesDone"].as_u64();
        self.count = value["entryCount"].as_u64();
        if let Some(current) = value["current"].as_str() {
            self.current = current.to_owned();
        }
        self.state = next_state.to_owned();
        self.can_pause = value["canPause"] == true;
        self.can_cancel = value["canCancel"] == true;
        self.samples.push_back((now, self.done));
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(at, _)| now - at > 3.0) {
            self.samples.pop_front();
        }
        while self.samples.len() > 32 {
            self.samples.pop_front();
        }
        self.graph.push_back(self.rate().unwrap_or(0.0));
        while self.graph.len() > 120 {
            self.graph.pop_front();
        }
    }

    fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| (self.done as f64 / total as f64).clamp(0.0, 1.0))
    }

    fn rate(&self) -> Option<f64> {
        if self.state == "paused" {
            return Some(0.0);
        }
        let (first_at, first_bytes) = self.samples.front()?;
        let (last_at, last_bytes) = self.samples.back()?;
        let elapsed = last_at - first_at;
        (elapsed >= 0.5 && last_bytes >= first_bytes).then(|| (last_bytes - first_bytes) as f64 / elapsed)
    }

    fn remaining_seconds(&self) -> Option<u64> {
        let total = self.total?;
        let rate = self.rate().filter(|rate| rate.is_finite() && *rate > 0.0)?;
        Some(((total.saturating_sub(self.done)) as f64 / rate).ceil().min(u64::MAX as f64) as u64)
    }

    fn remaining_items(&self) -> Option<u64> {
        self.count.zip(self.items).map(|(count, done)| count.saturating_sub(done))
    }
}

struct Inner {
    frame: AppWindow,
    stack: gtk::Stack,
    conflict: RefCell<Option<ConflictView>>,
    summary: gtk::Label,
    percent: gtk::Label,
    pause: gtk::Button,
    cancel: gtk::Button,
    compact: gtk::ProgressBar,
    expanded: gtk::Box,
    graph: gtk::DrawingArea,
    speed: gtk::Label,
    name: gtk::Label,
    remaining: gtk::Label,
    items: gtk::Label,
    error: gtk::Label,
    details: gtk::Button,
    details_label: gtk::Label,
    details_icon: gtk::Image,
    shown_details: Cell<bool>,
    control_pending: Cell<bool>,
    terminal: Cell<bool>,
    controlled: bool,
    operation: RefCell<String>,
    count: usize,
    source: String,
    destination: String,
    source_link: bool,
    destination_link: bool,
    started: Instant,
    metrics: RefCell<Metrics>,
    control_order: RefCell<ControlOrder>,
}

struct ConflictView {
    id: String,
    data: Value,
    root: gtk::Box,
    heading: gtk::Label,
    message: gtk::Label,
    source: gtk::Label,
    target: gtk::Label,
    apply: gtk::CheckButton,
    actions: Vec<(String, gtk::Button)>,
    error: gtk::Label,
}

fn conflict_choices(value: &Value) -> Vec<&'static str> {
    let mut choices: Vec<_> = ["retry", "replace", "skip", "keepBoth", "cancel"]
        .into_iter()
        .filter(|choice| {
            value["choices"]
                .as_array()
                .is_some_and(|allowed| allowed.iter().any(|v| v.as_str() == Some(*choice)))
        })
        .collect();
    if !choices.contains(&"cancel") {
        choices.push("cancel");
    }
    choices
}

fn conflict_key(choice: &str) -> &'static str {
    match choice {
        "retry" => "transferRetry",
        "replace" => "transferReplace",
        "skip" => "transferSkip",
        "keepBoth" => "transferKeepBoth",
        _ => "transferCancel",
    }
}

impl ConflictView {
    fn title(&self) -> &'static str {
        s(if self.data["kind"] == "error" {
            "transferProblemTitle"
        } else {
            "transferConflictTitle"
        })
    }

    fn relabel(&self) {
        self.heading.set_label(self.title());
        let name = self.data["target"]["name"].as_str().unwrap_or("");
        self.message.set_label(&if self.data["kind"] == "error" {
            self.data["error"]["message"].as_str().unwrap_or(s("errTitle")).to_owned()
        } else {
            f("transferConflictMessage", &[("name", name)])
        });
        for (label, item, key) in [
            (&self.source, &self.data["source"], "transferConflictSource"),
            (&self.target, &self.data["target"], "transferConflictTarget"),
        ] {
            let path = item["path"].as_str().unwrap_or("");
            let mut info = path.to_owned();
            if let Some(size) = item["size"].as_f64().filter(|size| size.is_finite() && *size >= 0.0) {
                info.push_str(&format!(" ({})", kit::format::bytes(size)));
            }
            if let Some(time) = item["mtime"].as_f64().filter(|time| time.is_finite() && *time > 0.0) {
                info.push_str(&format!(", {}", kit::format::file_time(time)));
            }
            label.set_label(&f(key, &[("info", &info)]));
            label.set_tooltip_text(Some(&info));
        }
        self.apply.set_label(Some(s("transferApplyAll")));
        for (choice, button) in &self.actions {
            button.set_label(s(conflict_key(choice)));
        }
    }
}

thread_local! {
    // Errors stay readable even after the job has left ops::JOBS. A caption close releases this last owner.
    static FAILED: RefCell<Vec<Rc<Inner>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
pub(crate) struct TransferWindow(Rc<Inner>);

fn line(name: &str, class: &str) -> gtk::Label {
    let label = kit::controls::label("", class, false);
    label.set_widget_name(name);
    label.set_xalign(0.0);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label.set_hexpand(true);
    label
}

fn captions(widget: &gtk::Widget) {
    match widget.widget_name().as_str() {
        "explorer-cap-max" => {
            widget.set_sensitive(false);
            widget.set_widget_name("explorer-transfer-maximize");
        }
        "explorer-cap-min" => widget.set_widget_name("explorer-transfer-minimize"),
        "explorer-cap-close" => widget.set_widget_name("explorer-transfer-close"),
        _ => {}
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        captions(&widget);
    }
}

impl TransferWindow {
    pub fn new(
        operation: &str,
        count: usize,
        source: String,
        destination: String,
        source_path: Option<String>,
        destination_path: Option<String>,
        control: Rc<dyn Fn(&str)>,
    ) -> Self {
        let frame = AppWindow::new(WindowOptions {
            key: String::new(),
            title: s("transferPreparing").to_owned(),
            icon: "copy".into(),
            width: 562,
            height: 206,
            min_width: 430,
            min_height: 206,
            ..Default::default()
        });
        frame.window().set_widget_name("explorer-transfer-window");
        frame.window().add_css_class("xp-transfer-window");
        frame.window().set_resizable(false);
        captions(frame.window().upcast_ref());
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.add_css_class("xp-transfer-body");
        body.set_vexpand(true);
        let summary = line("explorer-transfer-summary", "xp-transfer-summary");
        let source_link = source_path.is_some();
        let destination_link = destination_path.is_some();
        summary.connect_activate_link(move |_, uri| {
            // Fixed, in-process link roles. Never let a label launch an arbitrary URI or shell command.
            let path = match uri {
                "source" => source_path.as_ref(),
                "destination" => destination_path.as_ref(),
                _ => None,
            };
            if let Some(path) = path {
                kit::ctx().fire("files.open", serde_json::json!({"path": path}));
            }
            glib::Propagation::Stop
        });
        body.append(&summary);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        let percent = line("explorer-transfer-percent", "xp-transfer-percent");
        let pause = kit::controls::icon_button("pause", s("transferPause"), 32);
        pause.set_widget_name("explorer-transfer-pause");
        let cancel = kit::controls::icon_button("close", s("transferCancel"), 32);
        cancel.set_widget_name("explorer-transfer-cancel");
        row.append(&percent);
        row.append(&pause);
        row.append(&cancel);
        body.append(&row);
        let compact = gtk::ProgressBar::new();
        compact.set_widget_name("explorer-transfer-progress");
        compact.add_css_class("xp-transfer-progress");
        body.append(&compact);
        let expanded = gtk::Box::new(gtk::Orientation::Vertical, 10);
        expanded.set_visible(false);
        let chart = gtk::Overlay::new();
        let graph = gtk::DrawingArea::new();
        graph.set_widget_name("explorer-transfer-graph");
        graph.add_css_class("xp-transfer-graph");
        graph.set_content_height(98);
        graph.set_hexpand(true);
        chart.set_child(Some(&graph));
        let speed = line("explorer-transfer-speed", "xp-transfer-speed");
        speed.set_halign(gtk::Align::End);
        speed.set_valign(gtk::Align::End);
        speed.set_margin_end(8);
        speed.set_margin_bottom(8);
        chart.add_overlay(&speed);
        expanded.append(&chart);
        let detail_lines = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let name = line("explorer-transfer-name", "xp-transfer-detail");
        let remaining = line("explorer-transfer-remaining", "xp-transfer-detail");
        let items = line("explorer-transfer-items", "xp-transfer-detail");
        detail_lines.append(&name);
        detail_lines.append(&remaining);
        detail_lines.append(&items);
        expanded.append(&detail_lines);
        body.append(&expanded);
        let error = gtk::Label::new(None);
        error.set_widget_name("explorer-transfer-error");
        error.add_css_class("xp-transfer-error");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_visible(false);
        body.append(&error);
        root.append(&body);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.add_css_class("xp-transfer-footer");
        let details = gtk::Button::new();
        details.set_widget_name("explorer-transfer-details");
        details.add_css_class("xp-transfer-details-button");
        let details_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let circle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        circle.add_css_class("xp-transfer-chevron");
        circle.set_valign(gtk::Align::Center);
        let details_icon = kit::icons::glyph("chevron-down", 12);
        circle.append(&details_icon);
        let details_label = gtk::Label::new(None);
        details_row.append(&circle);
        details_row.append(&details_label);
        details.set_child(Some(&details_row));
        footer.append(&details);
        root.append(&footer);
        let stack = gtk::Stack::new();
        stack.add_named(&root, Some("progress"));
        frame.set_content(&stack);
        let this = Self(Rc::new(Inner {
            frame,
            stack,
            conflict: RefCell::new(None),
            summary,
            percent,
            pause,
            cancel,
            compact,
            expanded,
            graph,
            speed,
            name,
            remaining,
            items,
            error,
            details,
            details_label,
            details_icon,
            shown_details: Cell::new(false),
            control_pending: Cell::new(false),
            terminal: Cell::new(false),
            operation: RefCell::new(operation.to_owned()),
            controlled: matches!(operation, "copy" | "move" | "paste"),
            count,
            source,
            destination,
            source_link,
            destination_link,
            started: Instant::now(),
            metrics: RefCell::new(Metrics::default()),
            control_order: RefCell::new(ControlOrder::default()),
        }));
        let weak = Rc::downgrade(&this.0);
        this.0.details.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                let window = Self(inner);
                window.set_details(!window.0.shown_details.get());
            }
        });
        let weak = Rc::downgrade(&this.0);
        let pause_control = control.clone();
        this.0.pause.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                let action = if inner.metrics.borrow().state == "paused" {
                    "resume"
                } else {
                    "pause"
                };
                inner.control_pending.set(true);
                Self(inner).refresh();
                pause_control(action);
            }
        });
        let weak = Rc::downgrade(&this.0);
        let close_control = control.clone();
        this.0.cancel.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.control_pending.set(true);
                Self(inner).refresh();
                control("cancel");
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.frame.window().connect_close_request(move |window| {
            let Some(inner) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if inner.terminal.get() {
                FAILED.with(|windows| windows.borrow_mut().retain(|other| !Rc::ptr_eq(other, &inner)));
                glib::Propagation::Proceed
            } else if inner.controlled {
                if !inner.control_pending.get() && inner.metrics.borrow().can_cancel {
                    inner.control_pending.set(true);
                    Self(inner).refresh();
                    close_control("cancel");
                }
                glib::Propagation::Stop
            } else {
                window.set_visible(false);
                glib::Propagation::Stop
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.frame.add_shortcut("Escape", move || {
            if let Some(inner) = weak.upgrade() {
                inner.frame.window().close();
            }
            true
        });
        let weak = Rc::downgrade(&this.0);
        this.0.graph.set_draw_func(move |area, cr, width, height| {
            if let Some(inner) = weak.upgrade() {
                draw_graph(area, cr, width, height, &inner.metrics.borrow());
            }
        });
        let weak = Rc::downgrade(&this.0);
        kit::i18n::on_change(move || {
            if let Some(inner) = weak.upgrade() {
                Self(inner).refresh();
            }
        });
        this.refresh();
        this
    }

    pub fn show(&self, activate: bool) {
        if activate {
            self.0.frame.present();
        } else {
            self.0.frame.window().set_visible(true);
        }
    }

    pub fn update(&self, value: &Value) {
        self.0.control_order.borrow_mut().note_progress();
        if value["state"].as_str().is_some_and(|state| state != "waitingConflict") {
            self.dismiss_conflict();
        }
        if let Some(op @ ("copy" | "move" | "compress" | "extract")) = value["op"].as_str() {
            *self.0.operation.borrow_mut() = op.to_owned();
        }
        self.0.metrics.borrow_mut().observe(value, self.0.started.elapsed().as_secs_f64());
        self.0.control_pending.set(false);
        self.refresh();
        self.0.graph.queue_draw();
    }

    pub fn begin_control(&self) -> ControlStamp {
        self.0.control_order.borrow_mut().begin()
    }

    pub fn control_result(&self, value: &Value, stamp: ControlStamp) {
        if !self.0.control_order.borrow().current(stamp) {
            return;
        }
        if self.0.control_order.borrow().may_apply(stamp) {
            let mut metrics = self.0.metrics.borrow_mut();
            if let Some(state) = value["state"].as_str() {
                if metrics.state == "paused" || state == "paused" {
                    metrics.samples.clear();
                }
                metrics.state = state.to_owned();
            }
            metrics.can_pause = value["canPause"] == true;
            metrics.can_cancel = value["canCancel"] == true;
        }
        self.0.control_pending.set(false);
        self.0.error.set_visible(false);
        self.refresh();
    }

    pub fn control_error(&self, message: &str, stamp: ControlStamp) {
        if !self.0.control_order.borrow().current(stamp) {
            return;
        }
        self.0.control_pending.set(false);
        self.0.error.set_label(message);
        self.0.error.set_visible(true);
        self.refresh();
    }

    pub fn fail(&self, message: &str) {
        self.dismiss_conflict();
        self.0.terminal.set(true);
        self.0.error.set_label(message);
        self.0.error.set_visible(true);
        FAILED.with(|windows| windows.borrow_mut().push(self.0.clone()));
        self.set_details(true);
        self.refresh();
        self.0.frame.window().set_visible(true);
    }

    pub fn finish(&self) {
        self.0.terminal.set(true);
        self.0.frame.window().destroy();
    }

    pub fn show_conflict(&self, value: &Value, resolve: Rc<dyn Fn(&str, bool)>) {
        let Some(id) = value["conflictId"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        if self.0.conflict.borrow().as_ref().is_some_and(|view| view.id == id) {
            return;
        }
        self.dismiss_conflict();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("xp-transfer-conflict");
        root.set_widget_name("explorer-transfer-conflict");
        let heading = line("explorer-transfer-conflict-heading", "xp-transfer-percent");
        let message = gtk::Label::new(None);
        message.set_widget_name("explorer-transfer-conflict-message");
        message.set_xalign(0.0);
        message.set_wrap(true);
        root.append(&heading);
        root.append(&message);
        let source = line("explorer-transfer-conflict-source", "xp-transfer-conflict-info");
        let target = line("explorer-transfer-conflict-target", "xp-transfer-conflict-info");
        root.append(&source);
        root.append(&target);
        let apply = gtk::CheckButton::new();
        apply.set_widget_name("explorer-transfer-conflict-all");
        apply.set_visible(value["kind"] != "error");
        apply.set_hexpand(true);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.append(&apply);
        let mut actions = Vec::new();
        for choice in conflict_choices(value) {
            let button = kit::controls::button(
                "",
                if choice == "cancel" {
                    kit::controls::Variant::Standard
                } else {
                    kit::controls::Variant::Subtle
                },
            );
            button.set_widget_name(&format!("explorer-transfer-conflict-{choice}"));
            if choice != "cancel" {
                button.add_css_class("xp-transfer-conflict-action");
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                }
            }
            let weak = Rc::downgrade(&self.0);
            let expected_id = id.to_owned();
            let apply = apply.clone();
            let resolve = resolve.clone();
            button.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let conflict = inner.conflict.borrow();
                let Some(view) = conflict.as_ref().filter(|view| view.id == expected_id) else {
                    return;
                };
                for (_, button) in &view.actions {
                    button.set_sensitive(false);
                }
                view.apply.set_sensitive(false);
                view.error.set_visible(false);
                let all = choice != "cancel" && apply.is_active();
                drop(conflict);
                resolve(choice, all);
            });
            if choice == "cancel" {
                footer.append(&button);
            } else {
                root.append(&button);
            }
            actions.push((choice.to_owned(), button));
        }
        root.append(&footer);
        let error = gtk::Label::new(None);
        error.set_widget_name("explorer-transfer-conflict-error");
        error.add_css_class("xp-transfer-error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        root.append(&error);
        let view = ConflictView {
            id: id.to_owned(),
            data: value.clone(),
            root,
            heading,
            message,
            source,
            target,
            apply,
            actions,
            error,
        };
        view.relabel();
        self.0.stack.add_named(&view.root, Some("conflict"));
        *self.0.conflict.borrow_mut() = Some(view);
        self.0.stack.set_visible_child_name("conflict");
        self.0.frame.window().set_default_size(562, 440);
        self.refresh();
        self.0.frame.present();
    }

    pub fn conflict_error(&self, expected_id: &str, message: &str) {
        let conflict = self.0.conflict.borrow();
        if let Some(view) = conflict.as_ref().filter(|view| view.id == expected_id) {
            view.error.set_label(message);
            view.error.set_visible(true);
            for (_, button) in &view.actions {
                button.set_sensitive(true);
            }
            view.apply.set_sensitive(true);
        }
    }

    fn dismiss_conflict(&self) {
        let previous = self.0.conflict.borrow_mut().take();
        if let Some(view) = previous {
            self.0.stack.set_visible_child_name("progress");
            self.0.stack.remove(&view.root);
            self.0
                .frame
                .window()
                .set_default_size(562, if self.0.shown_details.get() { 360 } else { 206 });
        }
    }

    fn set_details(&self, show: bool) {
        self.0.shown_details.set(show);
        self.0.expanded.set_visible(show);
        self.0.compact.set_visible(!show);
        self.0.frame.window().set_default_size(562, if show { 360 } else { 206 });
        self.refresh();
    }

    fn refresh(&self) {
        let inner = &self.0;
        let m = inner.metrics.borrow();
        // Before the bounded scan answers, only the selected-root count is known.
        let count = m.count.and_then(|count| usize::try_from(count).ok()).unwrap_or(inner.count);
        let operation = inner.operation.borrow();
        let title = if inner.terminal.get() {
            s("transferFailed").to_owned()
        } else if matches!(m.state.as_str(), "paused" | "pausing" | "cancelling") {
            s(match m.state.as_str() {
                "paused" => "transferPaused",
                "pausing" => "transferPausing",
                _ => "transferCancelling",
            })
            .to_owned()
        } else if let Some(fraction) = m.fraction() {
            f("transferComplete", &[("percent", &format!("{:.0}", (fraction * 100.0).floor()))])
        } else {
            s("transferPreparing").to_owned()
        };
        inner.frame.set_title(&title);
        if let Some(conflict) = inner.conflict.borrow().as_ref() {
            conflict.relabel();
            inner.frame.set_title(conflict.title());
        }
        inner.percent.set_label(&title);
        let summary = if operation.as_str() == "compress" || operation.as_str() == "extract" {
            s(&format!("job_{operation}")).to_owned()
        } else {
            n(
                if operation.as_str() == "move" {
                    "transferMove"
                } else {
                    "transferCopy"
                },
                count,
                &[("source", &inner.source), ("dest", &inner.destination)],
            )
        };
        if operation.as_str() == "compress" || operation.as_str() == "extract" {
            inner.summary.set_label(&summary);
        } else {
            let source = location_markup(&inner.source, "source", inner.source_link);
            let destination = location_markup(&inner.destination, "destination", inner.destination_link);
            inner.summary.set_markup(&n(
                if operation.as_str() == "move" {
                    "transferMove"
                } else {
                    "transferCopy"
                },
                count,
                &[("source", &source), ("dest", &destination)],
            ));
        }
        inner.summary.set_tooltip_text(Some(&summary));
        inner.compact.set_fraction(m.fraction().unwrap_or(0.0));
        inner.pause.set_visible(inner.controlled);
        inner.cancel.set_visible(inner.controlled);
        inner.pause.set_sensitive(
            !inner.control_pending.get() && !inner.terminal.get() && m.can_pause && matches!(m.state.as_str(), "running" | "paused"),
        );
        inner
            .cancel
            .set_sensitive(!inner.control_pending.get() && !inner.terminal.get() && m.can_cancel && m.state != "cancelling");
        let paused = m.state == "paused";
        inner
            .pause
            .set_child(Some(&kit::icons::glyph(if paused { "play" } else { "pause" }, 12)));
        inner
            .pause
            .set_tooltip_text(Some(s(if paused { "transferResume" } else { "transferPause" })));
        inner.cancel.set_tooltip_text(Some(s("transferCancel")));
        for widget in [inner.compact.upcast_ref::<gtk::Widget>(), inner.graph.upcast_ref::<gtk::Widget>()] {
            if paused {
                widget.add_css_class("is-paused");
            } else {
                widget.remove_css_class("is-paused");
            }
        }
        let speed = m
            .rate()
            .map(kit::format::rate)
            .unwrap_or_else(|| s("transferCalculating").to_owned());
        inner.speed.set_label(&f("transferSpeed", &[("speed", &speed)]));
        inner.name.set_label(&f("transferName", &[("name", &util::basename(&m.current))]));
        inner.name.set_tooltip_text((!m.current.is_empty()).then_some(m.current.as_str()));
        let time = if paused {
            s("transferPaused").to_owned()
        } else {
            m.remaining_seconds()
                .map(|seconds| {
                    if seconds < 60 {
                        n("transferSeconds", seconds as usize, &[])
                    } else {
                        n("transferMinutes", seconds.div_ceil(60) as usize, &[])
                    }
                })
                .unwrap_or_else(|| s("transferCalculating").to_owned())
        };
        inner.remaining.set_label(&f("transferRemaining", &[("time", &time)]));
        let count = m
            .remaining_items()
            .map(|count| count.to_string())
            .unwrap_or_else(|| s("transferCalculating").to_owned());
        let bytes = m
            .total
            .map(|total| format!(" ({})", kit::format::bytes(total.saturating_sub(m.done) as f64)))
            .unwrap_or_default();
        inner
            .items
            .set_label(&f("transferItems", &[("count", &count.to_string()), ("bytes", &bytes)]));
        inner.details_label.set_label(s(if inner.shown_details.get() {
            "transferFewer"
        } else {
            "transferMore"
        }));
        inner.details_icon.set_paintable(
            kit::icons::glyph(if inner.shown_details.get() { "chevron-up" } else { "chevron-down" }, 12)
                .paintable()
                .as_ref(),
        );
    }
}

fn location_markup(name: &str, role: &str, linked: bool) -> String {
    let name = glib::markup_escape_text(name);
    if linked && matches!(role, "source" | "destination") {
        format!("<a href=\"{role}\">{name}</a>")
    } else {
        name.to_string()
    }
}

fn draw_graph(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: i32, height: i32, metrics: &Metrics) {
    let (width, height) = (width as f64, height as f64);
    let color = area.color();
    let (r, g, b) = (color.red() as f64, color.green() as f64, color.blue() as f64);
    let high_contrast = area
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|window| window.has_css_class("w11-hc"));
    if high_contrast {
        let background = kit::theme::token("bg-solid-base");
        cr.set_source_rgb(background.red() as f64, background.green() as f64, background.blue() as f64);
    } else {
        // The Windows graph keeps a light green/yellow field even when the surrounding Explorer is dark.
        cr.set_source_rgb(0.66 + r * 0.16, 0.72 + g * 0.18, 0.58 + b * 0.12);
    }
    cr.paint().ok();
    cr.set_source_rgba(r, g, b, 0.30);
    cr.set_line_width(1.0);
    for step in 1..10 {
        let x = (width * step as f64 / 10.0).floor() + 0.5;
        cr.move_to(x, 0.0);
        cr.line_to(x, height);
    }
    for step in 1..5 {
        let y = (height * step as f64 / 5.0).floor() + 0.5;
        cr.move_to(0.0, y);
        cr.line_to(width, y);
    }
    cr.stroke().ok();
    let peak = metrics.graph.iter().copied().fold(1.0_f64, f64::max) * 1.3;
    let history_width = width * metrics.fraction().unwrap_or(0.0);
    if metrics.graph.len() >= 2 {
        cr.move_to(0.0, height);
        for (index, rate) in metrics.graph.iter().enumerate() {
            let x = history_width * index as f64 / (metrics.graph.len() - 1) as f64;
            cr.line_to(x, (height - rate / peak * height).clamp(2.0, height - 2.0));
        }
        cr.line_to(history_width, height);
        cr.close_path();
        cr.set_source_rgb(r, g, b);
        cr.fill().ok();
    }
    cr.set_source_rgba(r, g, b, 0.65);
    cr.rectangle(0.5, 0.5, (width - 1.0).max(0.0), (height - 1.0).max(0.0));
    cr.stroke().ok();
}

#[cfg(test)]
mod tests {
    use super::{ControlOrder, Metrics};
    use serde_json::json;

    #[test]
    fn byte_progress_is_clamped_and_unknown_totals_do_not_invent_a_percentage() {
        let mut m = Metrics::default();
        m.observe(&json!({"done": 500, "total": null}), 0.0);
        assert_eq!(m.fraction(), None);
        m.observe(&json!({"done": 500, "total": 100}), 1.0);
        assert_eq!(m.fraction(), Some(1.0));
        m.observe(&json!({"done": 0, "total": 0}), 2.0);
        assert_eq!(m.fraction(), None);
    }

    #[test]
    fn speed_and_time_remaining_require_real_monotonic_samples() {
        let mut m = Metrics::default();
        m.observe(&json!({"done": 100, "total": 1000}), 0.0);
        assert_eq!(m.rate(), None);
        assert_eq!(m.remaining_seconds(), None);
        m.observe(&json!({"done": 300, "total": 1000}), 2.0);
        assert_eq!(m.rate(), Some(100.0));
        assert_eq!(m.remaining_seconds(), Some(7));
    }

    #[test]
    fn paused_duration_is_not_included_in_resumed_throughput() {
        let mut m = Metrics::default();
        m.observe(&json!({"done": 100, "total": 1000}), 0.0);
        m.observe(&json!({"done": 300, "total": 1000, "state": "paused"}), 2.0);
        assert_eq!(m.rate(), Some(0.0));
        m.observe(&json!({"done": 300, "total": 1000, "state": "running"}), 100.0);
        assert_eq!(m.rate(), None);
        m.observe(&json!({"done": 400, "total": 1000}), 101.0);
        assert_eq!(m.rate(), Some(100.0));
    }

    #[test]
    fn backwards_counters_reset_rate_and_history_memory_is_bounded() {
        let mut m = Metrics::default();
        m.observe(&json!({"done": 100, "total": 1000}), 0.0);
        m.observe(&json!({"done": 20, "total": 1000}), 2.0);
        assert_eq!(m.rate(), None);
        for i in 0..1000 {
            m.observe(&json!({"done": i, "total": 1000}), i as f64);
        }
        assert!(m.samples.len() <= 32);
        assert_eq!(m.graph.len(), 120);
    }

    #[test]
    fn folder_labels_are_escaped_and_only_fixed_link_roles_are_inserted() {
        assert_eq!(
            super::location_markup("A&B <folder>", "source", true),
            "<a href=\"source\">A&amp;B &lt;folder&gt;</a>"
        );
        assert_eq!(
            super::location_markup("various <folders>", "source", false),
            "various &lt;folders&gt;"
        );
        assert_eq!(super::location_markup("folder", "https://example.invalid/", true), "folder");
    }

    #[test]
    fn recursive_entry_counts_override_selected_roots_and_unknown_is_not_zero() {
        let mut m = Metrics::default();
        m.observe(&json!({"count": 1, "items": 0, "entryCount": null, "entriesDone": null}), 0.0);
        assert_eq!(m.count, None);
        assert_eq!(m.remaining_items(), None);
        m.observe(&json!({"count": 1, "items": 0, "entryCount": 984, "entriesDone": 268}), 1.0);
        assert_eq!(m.count, Some(984));
        assert_eq!(m.remaining_items(), Some(716));
        m.observe(&json!({"count": 1, "items": 1, "entryCount": 984, "entriesDone": 984}), 2.0);
        assert_eq!(m.remaining_items(), Some(0));
        // A legacy host or an aborted bounded scan has no exact recursive counters.
        m.observe(&json!({"count": 1, "items": 1}), 3.0);
        assert_eq!(m.count, None);
        assert_eq!(m.remaining_items(), None);
    }

    #[test]
    fn delayed_pause_response_cannot_replace_newer_paused_progress_or_resume_request() {
        let mut order = ControlOrder::default();
        let pause = order.begin();
        assert!(order.may_apply(pause));
        // Worker has already confirmed paused via SSE before HTTP delivers its older pausing response.
        order.note_progress();
        assert!(order.current(pause));
        assert!(!order.may_apply(pause));
        let resume = order.begin();
        assert!(!order.current(pause));
        assert!(order.may_apply(resume));
        order.note_progress();
        assert!(!order.may_apply(resume));
    }

    #[test]
    fn conflict_actions_follow_host_capabilities_and_never_invent_replace() {
        assert_eq!(
            super::conflict_choices(&json!({"choices": ["skip", "keepBoth", "cancel", "eraseTree"]})),
            vec!["skip", "keepBoth", "cancel"]
        );
        assert_eq!(
            super::conflict_choices(&json!({"choices": ["replace", "skip", "keepBoth", "cancel"]})),
            vec!["replace", "skip", "keepBoth", "cancel"]
        );
        assert_eq!(super::conflict_choices(&json!({"choices": []})), vec!["cancel"]);
        assert_eq!(
            super::conflict_choices(&json!({"kind": "error", "choices": ["retry", "skip", "cancel"]})),
            vec!["retry", "skip", "cancel"]
        );
        assert_eq!(
            super::conflict_choices(&json!({"kind": "error", "choices": ["skip", "cancel"]})),
            vec!["skip", "cancel"]
        );
    }
}
