//! Instellingen › Update en herstel › Updategeschiedenis
//! (`git show last-webkit:vendor/ui/js/apps/settings/pages/update-history.js`): Windows 11 24H2's Update history, one
//! expander per group with its count ("Updates (12)"), each entry a two-line row (name and version; "Geïnstalleerd op
//! {date}" or "Kon niet worden geïnstalleerd op {date}" with who installed it), newest first; update.history gives 200
//! entries at most per call, "Meer laden" asks for the ones before the oldest shown (its cursor). Under the groups,
//! Gerelateerde instellingen › Herstel opens Update en herstel at its backup and reset group.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use kit::controls::{self as c, Chevron, SettingsCard, SettingsExpander, SettingsGroup};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("update-history", update_history)];

/// settings.css "Updategeschiedenis".
pub const CSS: &str = r#"
.w11-settings-expander .w11-expander-inner > .st-updhist-list.w11-nested-other { padding: 0; }
.st-updhist-item { padding: 11px 16px 12px 16px; border-top: 1px solid var(--stroke-divider); }
.st-updhist-item:first-child { border-top-width: 0; }
.st-updhist-date { color: var(--text-2); font-size: var(--fs-caption); }
.st-updhist-more { margin-top: 12px; }
"#;

const PRODUCT: &str = "Win11 Shell";

/// The groups in Windows' order (updates first) and whether they start open.
const GROUPS: &[(&str, bool)] = &[("updates", true), ("releases", true), ("software", false)];

fn group_title(id: &str) -> &'static str {
    match id {
        "updates" => t("Updates", "Updates"),
        "releases" => t("Ubuntu-versies", "Ubuntu versions"),
        _ => t("Software", "Software"),
    }
}

/// "Geïnstalleerd op 25-10-2026 automatisch door Ubuntu" (the source is left out when unknown).
fn date_line(e: &Value) -> String {
    let date = kit::format::date(e["date"].as_f64().unwrap_or(f64::NAN), DateFormat::Short);
    let when = common::fill(
        if e["ok"] == true {
            t("Geïnstalleerd op {date}", "Successfully installed on {date}")
        } else {
            t("Kon niet worden geïnstalleerd op {date}", "Failed to install on {date}")
        },
        &[("date", &date)],
    );
    let by = match e["source"].as_str() {
        Some("ubuntu") => t("automatisch door Ubuntu", "automatically by Ubuntu").to_owned(),
        Some("us") => common::fill(t("via {product}", "via {product}"), &[("product", PRODUCT)]),
        Some("app") => t("via een app", "via an app").to_owned(),
        Some("terminal") => t("in een terminal (apt)", "in a terminal (apt)").to_owned(),
        _ => String::new(),
    };
    if by.is_empty() { when } else { format!("{when} {by}") }
}

struct State {
    p: Page,
    entries: RefCell<Vec<Value>>,
    more: Cell<bool>,
    open: RefCell<Vec<bool>>,
    root: gtk::Box,
}

fn group(st: &Rc<State>, i: usize, id: &str, mine: &[&Value]) -> gtk::Widget {
    let exp = SettingsExpander::new("", &format!("{} ({})", group_title(id), mine.len()), "");
    exp.header_button().set_widget_name(&format!("history-{id}"));
    exp.set_expanded(st.open.borrow().get(i).copied().unwrap_or(false));
    let weak = Rc::downgrade(st);
    exp.connect_toggled(move |o| {
        if let Some(st) = weak.upgrade()
            && let Some(slot) = st.open.borrow_mut().get_mut(i)
        {
            *slot = o;
        }
    });
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("st-updhist-list");
    for e in mine {
        let item = gtk::Box::new(gtk::Orientation::Vertical, 2);
        item.add_css_class("st-updhist-item");
        if e["ok"] != true {
            item.add_css_class("st-failed");
        }
        let name: Vec<String> = [&e["name"], &e["version"]].iter().map(|v| common::value_key(v)).filter(|s| !s.is_empty()).collect();
        let n = c::label(&name.join(" "), "st-updhist-name", true);
        n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        n.set_selectable(true);
        item.append(&n);
        item.append(&c::label(&date_line(e), "st-updhist-date", true));
        list.append(&item);
    }
    exp.add(&list);
    exp.widget().clone().upcast()
}

/// The groups and "Meer laden"; the button, when there is one.
fn paint(st: &Rc<State>) -> Option<gtk::Button> {
    c::clear(&st.root);
    let entries = st.entries.borrow().clone();
    if entries.is_empty() {
        st.root.append(&c::label(t("Er is nog geen updategeschiedenis.", "There’s no update history yet."), "st-lead", true));
        return None;
    }
    let groups = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for (i, (id, _)) in GROUPS.iter().enumerate() {
        let mine: Vec<&Value> = entries.iter().filter(|e| e["group"] == *id).collect();
        if !mine.is_empty() {
            groups.append(&group(st, i, id, &mine));
        }
    }
    st.root.append(&groups);
    if !st.more.get() {
        return None;
    }
    let btn = c::button(t("Meer laden", "Load more"), c::Variant::Standard);
    btn.set_widget_name("history-more");
    let weak = Rc::downgrade(st);
    btn.connect_clicked(move |b| {
        if let Some(st) = weak.upgrade() {
            load_more(&st, b);
        }
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("st-updhist-more");
    row.append(&btn);
    st.root.append(&row);
    Some(btn)
}

fn load_more(st: &Rc<State>, btn: &gtk::Button) {
    let focused = btn.has_focus();
    btn.set_sensitive(false);
    // the cursor (an older host: the date in ms, the design's form)
    let args = match st.entries.borrow().last() {
        Some(last) => json!({ "before": if last["cursor"].is_null() { last["date"].clone() } else { last["cursor"].clone() } }),
        None => json!({}),
    };
    let fut = st.p.call("update.history", args);
    let (st, btn) = (st.clone(), btn.clone());
    st.p.clone().spawn(async move {
        let r = fut.await;
        if !st.p.alive() {
            return;
        }
        match r {
            Err(e) => {
                st.p.error(&e, None);
                btn.set_sensitive(true);
            }
            Ok(v) => {
                st.entries.borrow_mut().extend(v["entries"].as_array().cloned().unwrap_or_default());
                st.more.set(v["more"] == true);
                // keyboard focus stays on "Meer laden" while there is more (paint builds the list again)
                if let Some(next) = paint(&st)
                    && focused
                {
                    next.grab_focus();
                }
            }
        }
    });
}

fn update_history(p: Page) -> PageFuture {
    Box::pin(async move {
        let first = p.call("update.history", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let v = match first {
            Ok(v) => v,
            Err(e) => {
                let text = if e.code == "unknown_method" {
                    ct::unavailable_here().to_owned()
                } else if e.message.is_empty() {
                    ct::failed().to_owned()
                } else {
                    e.message.clone()
                };
                return vec![c::label(&text, "st-lead", true).upcast()];
            }
        };
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("st-updhist");
        let st = Rc::new(State {
            p: p.clone(),
            entries: RefCell::new(v["entries"].as_array().cloned().unwrap_or_default()),
            more: Cell::new(v["more"] == true),
            open: RefCell::new(GROUPS.iter().map(|(_, o)| *o).collect()),
            root,
        });
        paint(&st);
        // Windows' Related settings: Recovery opens Update en herstel at its backup and reset group
        let related = SettingsGroup::new(ct::related(), "");
        let rec = SettingsCard::clickable(
            "restart",
            t("Herstel", "Recovery"),
            t("Back-up van instellingen, alles terugzetten", "Settings backup, reset everything"),
            Chevron::Right,
        );
        if let Some(b) = rec.button() {
            b.set_widget_name("history-recovery");
        }
        let pp = p.clone();
        rec.connect_clicked(move || pp.go_with("update", json!({ "section": "recovery" })));
        related.add(rec.widget());
        let root: gtk::Widget = st.root.clone().upcast();
        p.on_cleanup(move || drop(st));
        vec![root, related.widget().clone().upcast()]
    })
}
