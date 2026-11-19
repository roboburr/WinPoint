//! GNOME's settings on a page (notes/claude/settings-parity.md §6.3): the controls of a page bound to GSettings keys
//! through the host's `gsettings.*` (host/src/services/gsettings.rs), as `common::setting_toggle` binds WinPoint's
//! own settings.
//!
//! ```ignore
//! const SPEED: GKey = gs::key("org.gnome.desktop.peripherals.mouse", "speed");
//! let gs = Gs::load(&p, &[LEFT, SPEED]).await;      // one gsettings.get per page
//! if let Some(card) = gs.toggle_card(&LEFT, "mouse", t("Linkshandig", "Left-handed"), "", false) { .. }
//! let slider = gs.slider(&SPEED, 1.0, 20.0, 1.0, |v| v.as_f64().unwrap_or(0.0) * 10.0 + 10.0, |n| json!((n - 10.0) / 10.0));
//! ```
//!
//! * Writes are optimistic: the control changes at once, a refused write reverts it with the error toast (the
//!   "Testmodus" toast in a dry run).
//! * A key that is not writable (lockdown) makes its control insensitive with the tooltip "Sommige van deze
//!   instellingen worden beheerd door je organisatie."; a key WinPoint's look owns becomes a link to the WinPoint
//!   page that sets it (`*_card`) or an insensitive control; an unavailable key (schema or key not installed, not
//!   allowlisted) gives no card at all.
//! * Controls follow `gsettings.changed`, also for a change made elsewhere. The page keeps its `Gs` until it goes
//!   (`Gs::load` hands a clone to the page's cleanups), so a page that only calls [`Gs::follow`] keeps following
//!   outside changes after its own clones drop.
//! * [`Gs::set_values`] writes several keys in one `gsettings.set`, optimistic for each (a touchpad's scroll method,
//!   a set of related accessibility keys).
//! * Every bound control is named for the GNOME harness (`set_widget_name`, [`control_name`]): `st-<page>-<key>`, or
//!   `st-<page>-<part>-<key>` when the page loads that key name from more than one schema or path (the proxy page's
//!   `st-proxy-https-host`, a second tablet's `st-pen-056a-0358-area`).
use crate::pages::Page;
use kit::controls::{self as c, Dropdown, DropdownOption, RadioGroup, SettingsCard, Slider, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One GSettings key: a schema with its own path, or a relocatable schema at `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct GKey {
    pub schema: &'static str,
    pub key: &'static str,
    pub path: Option<String>,
}

/// A key of a schema with its own path.
pub const fn key(schema: &'static str, key: &'static str) -> GKey {
    GKey { schema, key, path: None }
}

/// A key of a relocatable schema at `path` (e.g. a tablet's `/org/gnome/desktop/peripherals/tablets/056a:0357/`).
pub fn at(schema: &'static str, path: &str, key: &'static str) -> GKey {
    GKey { schema, key, path: Some(path.to_owned()) }
}

impl GKey {
    fn slot(&self) -> String {
        format!("{}|{}|{}", self.schema, self.path.as_deref().unwrap_or(""), self.key)
    }

    fn args(&self) -> Value {
        let mut a = json!({ "schema": self.schema, "key": self.key });
        if let Some(path) = &self.path {
            a["path"] = json!(path);
        }
        a
    }
}

/// The lock tooltip (Windows: Sommige van deze instellingen worden beheerd door je organisatie.).
pub fn locked_text() -> &'static str {
    t("Sommige van deze instellingen worden beheerd door je organisatie.", "Some of these settings are managed by your organisation.")
}

/// The page that sets a WinPoint-owned GNOME key (the item's `setting`).
fn owner_page(setting: &str) -> &'static str {
    if setting.starts_with("wallpaper") { "background" } else { "colors" }
}

type Listener = Rc<dyn Fn(&Value)>;

/// A widget name's part: lowercase letters and digits, the rest as '-' (`056a:0357` → `056a-0357`).
fn name_part(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}

/// The harness name of the control bound to `k` on page `page` whose `Gs::load` loaded `keys` (module docs): the
/// key name alone while no other loaded key has it, else with the last segment of the schema id when the schemas
/// differ, or of the path when one relocatable schema is loaded at two paths.
pub fn control_name(page: &str, keys: &[GKey], k: &GKey) -> String {
    let same_key: Vec<&GKey> = keys.iter().filter(|o| o.key == k.key && *o != k).collect();
    if same_key.is_empty() {
        return format!("st-{page}-{}", k.key);
    }
    let part = if same_key.iter().any(|o| o.schema == k.schema) {
        let path = k.path.as_deref().unwrap_or("");
        path.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_owned()
    } else {
        k.schema.rsplit('.').next().unwrap_or(k.schema).to_owned()
    };
    format!("st-{page}-{}-{}", name_part(&part), k.key)
}

struct Inner {
    p: Page,
    keys: Vec<GKey>,
    items: RefCell<HashMap<String, Value>>,
    listeners: RefCell<Vec<(String, Listener)>>,
}

/// The GSettings items of one page (cheap to clone).
#[derive(Clone)]
pub struct Gs(Rc<Inner>);

impl Gs {
    /// Read `keys` in one `gsettings.get` and follow their changes while the page is shown.
    pub async fn load(p: &Page, keys: &[GKey]) -> Gs {
        let gs = Gs(Rc::new(Inner {
            p: p.clone(),
            keys: keys.to_vec(),
            items: RefCell::new(HashMap::new()),
            listeners: RefCell::new(Vec::new()),
        }));
        let list: Vec<Value> = keys.iter().map(GKey::args).collect();
        if let Some(v) = p.get("gsettings.get", json!({ "keys": list })).await {
            for item in v["items"].as_array().into_iter().flatten() {
                gs.store(item);
            }
        }
        let weak = Rc::downgrade(&gs.0);
        p.on("gsettings.changed", move |data| {
            if let Some(inner) = weak.upgrade() {
                Gs(inner).changed(&data["item"]);
            }
        });
        // the page keeps its items while it is shown (the cycle Page -> cleanups -> Gs -> Page ends when the page's
        // cleanups run), so a page that only follows keys does not lose the outside changes once its clones drop
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        gs
    }

    fn slot_of(item: &Value) -> String {
        let schema = item["schema"].as_str().unwrap_or("");
        let path = item["path"].as_str().unwrap_or("");
        let key = item["key"].as_str().unwrap_or("");
        // a fixed-path schema is asked without a path but answered with its own: match both spellings
        format!("{schema}|{path}|{key}")
    }

    fn store(&self, item: &Value) {
        if !item.is_object() {
            return;
        }
        let mut items = self.0.items.borrow_mut();
        items.insert(Self::slot_of(item), item.clone());
        let schema = item["schema"].as_str().unwrap_or("");
        let key = item["key"].as_str().unwrap_or("");
        items.insert(format!("{schema}||{key}"), item.clone());
    }

    fn changed(&self, item: &Value) {
        if !item.is_object() {
            return;
        }
        let slots = [Self::slot_of(item), format!("{}||{}", item["schema"].as_str().unwrap_or(""), item["key"].as_str().unwrap_or(""))];
        if !slots.iter().any(|s| self.0.items.borrow().contains_key(s)) {
            return;
        }
        self.store(item);
        let listeners: Vec<Listener> =
            self.0.listeners.borrow().iter().filter(|(s, _)| slots.contains(s)).map(|(_, f)| f.clone()).collect();
        for f in listeners {
            f(&item["value"]);
        }
    }

    /// The page these items belong to.
    pub fn page(&self) -> &Page {
        &self.0.p
    }

    /// The host's item of a key (Null when it was not loaded).
    pub fn item(&self, k: &GKey) -> Value {
        self.0.items.borrow().get(&k.slot()).cloned().unwrap_or(Value::Null)
    }

    /// The key's value (Null when unknown).
    pub fn value(&self, k: &GKey) -> Value {
        self.item(k)["value"].clone()
    }

    /// Whether the key exists here and is allowlisted (else its card is left out).
    pub fn available(&self, k: &GKey) -> bool {
        let item = self.item(k);
        item["available"] == true && item["allowed"] == true
    }

    /// Whether a write can be taken: available, not locked, in a writing mode.
    pub fn writable(&self, k: &GKey) -> bool {
        let item = self.item(k);
        self.available(k) && item["writable"] == true && matches!(item["mode"].as_str(), Some("write" | "mirrored"))
    }

    /// The WinPoint setting that owns the key (WinPoint's look), if any.
    pub fn owned(&self, k: &GKey) -> Option<String> {
        let item = self.item(k);
        (item["mode"] == "owned").then(|| item["setting"].as_str().unwrap_or("").to_owned())
    }

    /// Run `f(value)` when the key changes (gsettings.changed), while the page is shown.
    pub fn follow(&self, k: &GKey, f: impl Fn(&Value) + 'static) {
        self.0.listeners.borrow_mut().push((k.slot(), Rc::new(f)));
    }

    /// Write one key: optimistic (the cache changes at once), `revert()` and the error toast when it is refused.
    pub fn set(&self, k: &GKey, value: Value, revert: impl FnOnce() + 'static) {
        let previous = self.item(k);
        if let Some(mut item) = previous.as_object().cloned().map(Value::Object) {
            item["value"] = value.clone();
            self.store(&item);
        }
        let mut args = json!({ "schema": k.schema, "values": { k.key: value } });
        if let Some(path) = &k.path {
            args["path"] = json!(path);
        }
        let wait = self.0.p.run_with("gsettings.set", args, None, false);
        let (gs, k) = (self.clone(), k.clone());
        self.0.p.spawn(async move {
            match wait.await {
                Some(result) => {
                    for item in result["items"].as_array().into_iter().flatten() {
                        gs.store(item);
                    }
                }
                None => {
                    gs.store(&previous);
                    revert();
                    drop(k);
                }
            }
        });
    }

    /// Write several keys in one `gsettings.set` (one write per schema and path, in the order given): optimistic for
    /// each key (the cache changes at once); a refused write restores every key's cache and calls `revert()` once,
    /// after the error toast, as [`Gs::set`] does for one key.
    pub fn set_values(&self, items: &[(&GKey, Value)], revert: impl FnOnce() + 'static) {
        if items.is_empty() {
            return;
        }
        let previous: Vec<Value> = items.iter().map(|(k, _)| self.item(k)).collect();
        for ((_, value), before) in items.iter().zip(&previous) {
            if let Some(mut item) = before.as_object().cloned().map(Value::Object) {
                item["value"] = value.clone();
                self.store(&item);
            }
        }
        let wait = self.0.p.run_with("gsettings.set", write_args(items), None, false);
        let gs = self.clone();
        self.0.p.spawn(async move {
            match wait.await {
                Some(result) => {
                    for item in result["items"].as_array().into_iter().flatten() {
                        gs.store(item);
                    }
                }
                None => {
                    for item in &previous {
                        gs.store(item);
                    }
                    revert();
                }
            }
        });
    }

    fn name(&self, w: &impl IsA<gtk::Widget>, k: &GKey) {
        w.as_ref().set_widget_name(&control_name(self.0.p.id(), &self.0.keys, k));
    }

    fn lock(&self, w: &impl IsA<gtk::Widget>, k: &GKey) {
        let writable = self.writable(k);
        w.as_ref().set_sensitive(writable);
        if !writable && self.available(k) {
            w.as_ref().set_tooltip_text(Some(if self.owned(k).is_some() {
                t("WinPoint regelt deze instelling zelf.", "WinPoint manages this setting itself.")
            } else {
                locked_text()
            }));
        }
    }

    fn is_on(&self, k: &GKey, invert: bool) -> bool {
        (self.value(k) == true) != invert
    }

    /// A switch for a boolean key (`invert`: the switch shows !value).
    pub fn toggle(&self, k: &GKey, invert: bool) -> ToggleSwitch {
        let sw = ToggleSwitch::new(self.is_on(k, invert));
        self.name(sw.switch(), k);
        self.lock(sw.switch(), k);
        if !self.writable(k) {
            sw.set_sensitive(false);
        }
        let (gs, k2, sw2) = (self.clone(), k.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let (sw3, gs3, k3) = (sw2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, json!(on != invert), move || sw3.set_active(gs3.is_on(&k3, invert)));
        });
        let sw2 = sw.clone();
        self.follow(k, move |v| sw2.set_active((*v == true) != invert));
        sw
    }

    /// A check box for a boolean key.
    pub fn checkbox(&self, k: &GKey, label: &str, invert: bool) -> gtk::CheckButton {
        let cb = c::checkbox(label);
        cb.set_active(self.is_on(k, invert));
        self.name(&cb, k);
        self.lock(&cb, k);
        let silent = Rc::new(std::cell::Cell::new(false));
        let (gs, k2, s2) = (self.clone(), k.clone(), silent.clone());
        cb.connect_toggled(move |b| {
            if s2.get() {
                return;
            }
            let (b2, s3, gs3, k3) = (b.clone(), s2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, json!(b.is_active() != invert), move || {
                s3.set(true);
                b2.set_active(gs3.is_on(&k3, invert));
                s3.set(false);
            });
        });
        let cb2 = cb.clone();
        self.follow(k, move |v| {
            silent.set(true);
            cb2.set_active((*v == true) != invert);
            silent.set(false);
        });
        cb
    }

    /// A drop-down; `options` are (value as JSON, label), the value written back as that JSON.
    pub fn dropdown(&self, k: &GKey, options: Vec<(Value, String)>) -> Dropdown {
        let current = crate::common::value_key(&self.value(k));
        let dd = Dropdown::new(options.iter().map(|(v, l)| DropdownOption::new(&crate::common::value_key(v), l)).collect(), Some(&current));
        self.name(dd.widget(), k);
        self.lock(dd.widget(), k);
        let values: Rc<HashMap<String, Value>> = Rc::new(options.iter().map(|(v, _)| (crate::common::value_key(v), v.clone())).collect());
        let (gs, k2, dd2) = (self.clone(), k.clone(), dd.clone());
        dd.connect_changed(move |choice| {
            let value = values.get(choice).cloned().unwrap_or_else(|| json!(choice));
            let (dd3, gs3, k3) = (dd2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, value, move || dd3.set_selected(Some(&crate::common::value_key(&gs3.value(&k3)))));
        });
        let dd2 = dd.clone();
        self.follow(k, move |v| dd2.set_selected(Some(&crate::common::value_key(v))));
        dd
    }

    /// Radio buttons; `options` as for [`Gs::dropdown`].
    pub fn radio(&self, k: &GKey, options: Vec<(Value, String)>) -> RadioGroup {
        let keyed: Vec<(String, String)> = options.iter().map(|(v, l)| (crate::common::value_key(v), l.clone())).collect();
        let refs: Vec<(&str, &str)> = keyed.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        let current = crate::common::value_key(&self.value(k));
        let rg = RadioGroup::new(&refs, Some(&current), None, false);
        self.name(rg.widget(), k);
        self.lock(rg.widget(), k);
        let values: Rc<HashMap<String, Value>> = Rc::new(options.iter().map(|(v, _)| (crate::common::value_key(v), v.clone())).collect());
        let (gs, k2, rg2) = (self.clone(), k.clone(), rg.clone());
        rg.connect_changed(move |choice| {
            let value = values.get(choice).cloned().unwrap_or_else(|| json!(choice));
            let (rg3, gs3, k3) = (rg2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, value, move || rg3.set_value(&crate::common::value_key(&gs3.value(&k3))));
        });
        let rg2 = rg.clone();
        self.follow(k, move |v| rg2.set_value(&crate::common::value_key(v)));
        rg
    }

    /// A slider written when released; `to_ui` maps GNOME's value to the slider, `from_ui` back (the mouse's 1-20
    /// steps and speed -1..1, night light's 0-100 and Kelvin, ...).
    pub fn slider(&self, k: &GKey, min: f64, max: f64, step: f64, to_ui: fn(&Value) -> f64, from_ui: fn(f64) -> Value) -> Slider {
        let s = Slider::new(min, max, step, to_ui(&self.value(k)).clamp(min, max));
        s.set_value_visible(false);
        self.name(s.scale(), k);
        self.lock(s.scale(), k);
        if !self.writable(k) {
            s.set_sensitive(false);
        }
        let (gs, k2, s2) = (self.clone(), k.clone(), s.clone());
        s.connect_committed(move |n| {
            let (s3, gs3, k3) = (s2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, from_ui(n), move || s3.set_value(to_ui(&gs3.value(&k3)).clamp(min, max)));
        });
        let s2 = s.clone();
        self.follow(k, move |v| s2.set_value(to_ui(v).clamp(min, max)));
        s
    }

    /// A number box for an integer key (Windows' number field: the workspaces count, a dwell threshold).
    pub fn number(&self, k: &GKey, min: f64, max: f64, step: f64) -> gtk::SpinButton {
        let sb = c::number_box(min, max, step, self.value(k).as_f64().unwrap_or(min), 0);
        self.name(&sb, k);
        self.lock(&sb, k);
        let silent = Rc::new(std::cell::Cell::new(false));
        let (gs, k2, s2) = (self.clone(), k.clone(), silent.clone());
        sb.connect_value_changed(move |b| {
            if s2.get() {
                return;
            }
            let (b2, s3, gs3, k3) = (b.clone(), s2.clone(), gs.clone(), k2.clone());
            gs.set(&k2, json!(b.value().round() as i64), move || {
                s3.set(true);
                b2.set_value(gs3.value(&k3).as_f64().unwrap_or(min));
                s3.set(false);
            });
        });
        let sb2 = sb.clone();
        self.follow(k, move |v| {
            silent.set(true);
            sb2.set_value(v.as_f64().unwrap_or(min));
            silent.set(false);
        });
        sb
    }

    /// The card of a WinPoint-owned key: a link to the WinPoint page that sets it.
    fn owned_card(&self, k: &GKey, icon: &str, title: &str, description: &str) -> Option<SettingsCard> {
        let setting = self.owned(k)?;
        let page = owner_page(&setting);
        let card = crate::common::link_card_with(&self.0.p, page, Some(title), Some(description), Some(icon));
        if let Some(b) = card.button() {
            b.set_widget_name(&format!("st-{}-{}", self.0.p.id(), k.key));
        }
        Some(card)
    }

    /// A settings card with a switch; None when the key is not available here.
    pub fn toggle_card(&self, k: &GKey, icon: &str, title: &str, description: &str, invert: bool) -> Option<SettingsCard> {
        if !self.available(k) {
            return None;
        }
        if let Some(card) = self.owned_card(k, icon, title, description) {
            return Some(card);
        }
        let card = SettingsCard::new(icon, title, description);
        card.set_control(self.toggle(k, invert).widget());
        Some(card)
    }

    /// A settings card with a drop-down; None when the key is not available here.
    pub fn dropdown_card(&self, k: &GKey, icon: &str, title: &str, description: &str, options: Vec<(Value, String)>) -> Option<SettingsCard> {
        if !self.available(k) {
            return None;
        }
        if let Some(card) = self.owned_card(k, icon, title, description) {
            return Some(card);
        }
        let card = SettingsCard::new(icon, title, description);
        card.set_control(self.dropdown(k, options).widget());
        Some(card)
    }
}

/// The `gsettings.set` arguments of [`Gs::set_values`]: the single form for one schema and path, else `writes` with
/// one entry per schema and path in the order of their first key.
fn write_args(items: &[(&GKey, Value)]) -> Value {
    let mut writes: Vec<Value> = Vec::new();
    for (k, value) in items {
        let at = writes.iter().position(|w| w["schema"] == k.schema && w["path"].as_str() == k.path.as_deref());
        let w = match at {
            Some(i) => &mut writes[i],
            None => {
                let mut w = json!({ "schema": k.schema, "values": {} });
                if let Some(path) = &k.path {
                    w["path"] = json!(path);
                }
                writes.push(w);
                writes.last_mut().expect("just pushed")
            }
        };
        w["values"][k.key] = value.clone();
    }
    if writes.len() == 1 { writes.remove(0) } else { json!({ "writes": writes }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_of_the_same_key_name_get_names_of_their_own() {
        // review of B0, finding 18: the proxy page binds host and port of four schemas
        let proxy: Vec<GKey> = ["http", "https", "ftp", "socks"]
            .iter()
            .flat_map(|s| {
                let schema: &'static str = match *s {
                    "http" => "org.gnome.system.proxy.http",
                    "https" => "org.gnome.system.proxy.https",
                    "ftp" => "org.gnome.system.proxy.ftp",
                    _ => "org.gnome.system.proxy.socks",
                };
                [key(schema, "host"), key(schema, "port")]
            })
            .chain([key("org.gnome.system.proxy", "mode")])
            .collect();
        assert_eq!(control_name("proxy", &proxy, &proxy[2]), "st-proxy-https-host");
        assert_eq!(control_name("proxy", &proxy, &proxy[7]), "st-proxy-socks-port");
        assert_eq!(control_name("proxy", &proxy, &proxy[8]), "st-proxy-mode", "a key name of its own stays short");
        let names: std::collections::HashSet<String> = proxy.iter().map(|k| control_name("proxy", &proxy, k)).collect();
        assert_eq!(names.len(), proxy.len(), "every control has its own name");
        // one relocatable schema at two paths: the path's last segment
        let tablet = "org.gnome.desktop.peripherals.tablet";
        let two = [at(tablet, "/org/gnome/desktop/peripherals/tablets/056a:0357/", "area"), at(tablet, "/org/gnome/desktop/peripherals/tablets/056a:0358/", "area")];
        assert_eq!(control_name("pen", &two, &two[1]), "st-pen-056a-0358-area");
        assert_eq!(control_name("pen", &two[..1], &two[0]), "st-pen-area");
        // multitasking: current-workspace-only of both switchers
        let switchers = [key("org.gnome.shell.app-switcher", "current-workspace-only"), key("org.gnome.shell.window-switcher", "current-workspace-only")];
        assert_eq!(control_name("multitasking", &switchers, &switchers[0]), "st-multitasking-app-switcher-current-workspace-only");
    }

    #[test]
    fn several_keys_go_in_one_write_per_schema_and_path() {
        let touchpad = "org.gnome.desktop.peripherals.touchpad";
        let (edge, two) = (key(touchpad, "edge-scrolling-enabled"), key(touchpad, "two-finger-scrolling-enabled"));
        assert_eq!(
            write_args(&[(&edge, json!(false)), (&two, json!(true))]),
            json!({ "schema": touchpad, "values": { "edge-scrolling-enabled": false, "two-finger-scrolling-enabled": true } }),
            "one schema: the single form"
        );
        let a11y = key("org.gnome.desktop.a11y.interface", "high-contrast");
        let tablet = at("org.gnome.desktop.peripherals.tablet", "/org/gnome/desktop/peripherals/tablets/056a:0357/", "area");
        let args = write_args(&[(&edge, json!(true)), (&a11y, json!(true)), (&two, json!(false)), (&tablet, json!([0.0, 0.0, 0.0, 0.0]))]);
        let writes = args["writes"].as_array().expect("writes");
        assert_eq!(writes.len(), 3, "{args}");
        assert_eq!(writes[0], json!({ "schema": touchpad, "values": { "edge-scrolling-enabled": true, "two-finger-scrolling-enabled": false } }));
        assert_eq!(writes[1]["schema"], "org.gnome.desktop.a11y.interface");
        assert!(writes[1].get("path").is_none(), "a fixed-path schema is written without a path");
        assert_eq!(writes[2]["path"], "/org/gnome/desktop/peripherals/tablets/056a:0357/");
        assert!(args.get("schema").is_none(), "never both forms");
    }
}
