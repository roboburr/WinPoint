//! Instellingen › Systeem › Beeldscherm (vendor/ui/js/apps/settings/pages/display.js): monitors (arrangement,
//! scale, resolution, refresh rate with the Windows "Deze beeldscherminstellingen behouden?" countdown),
//! brightness and night light. Under GNOME display.set and the night light belong to GNOME's own Settings app
//! (D-G2): the host reports the monitors read-only and the page shows the values greyed out.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::widgets::{self, Clamp};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Slider, Toast, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("display", display)];
pub const CSS: &str = "";

const SCALES: &[f64] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0];

/// en "2560 × 1440 (Recommended)" needs about 185 px of text and is cut at 220 (GE1a width audit); nl keeps 220.
fn dd_width() -> i32 {
    if kit::i18n::en() { 240 } else { 220 }
}

fn recommended(v: &str) -> String {
    common::fill(t("{v} (aanbevolen)", "{v} (Recommended)"), &[("v", v)])
}

fn hz(n: f64) -> String {
    common::fill(t("{n} Hz", "{n} Hz"), &[("n", &format!("{}", n.round() as i64))])
}

#[derive(Clone, Debug, PartialEq)]
struct Mode {
    w: i64,
    h: i64,
    hz: f64,
    raw: String,
}

/// "2560x1440@59.951" -> Mode (display.js parseMode).
fn parse_mode(m: &str) -> Option<Mode> {
    let (size, rate) = match m.split_once('@') {
        Some((s, r)) => (s, Some(r)),
        None => (m, None),
    };
    let (w, h) = size.split_once('x')?;
    let digits = |s: &str| s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
    let w: i64 = digits(w).parse().ok().filter(|_| w.chars().all(|c| c.is_ascii_digit()))?;
    let h: i64 = digits(h).parse().ok()?;
    let hz = rate
        .map(|r| r.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>())
        .and_then(|r| r.parse::<f64>().ok())
        .unwrap_or(60.0);
    Some(Mode { w, h, hz, raw: m.to_owned() })
}

fn modes(mon: &Value) -> Vec<Mode> {
    mon["modes"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().and_then(parse_mode)).collect()).unwrap_or_default()
}

fn num(v: &Value, key: &str, default: f64) -> f64 {
    v[key].as_f64().unwrap_or(default)
}

/// The monitor's current mode string (display.js currentMode).
fn current_mode(mon: &Value) -> String {
    let (w, h, refresh) = (num(mon, "width", 0.0) as i64, num(mon, "height", 0.0) as i64, num(mon, "refresh", 60.0));
    let mut same: Vec<Mode> = modes(mon).into_iter().filter(|m| m.w == w && m.h == h).collect();
    if same.is_empty() {
        return format!("{w}x{h}@{}", refresh.round() as i64);
    }
    same.sort_by(|a, b| (a.hz - refresh).abs().total_cmp(&(b.hz - refresh).abs()));
    same[0].raw.clone()
}

/// Windows' 15 s confirmation (display.js keepChangesDialog): `done(true)` to keep the change.
pub fn keep_changes_dialog(p: &Page, seconds: i64, done: impl FnOnce(bool) + 'static) {
    let body = |n: i64| {
        common::plural(
            n,
            t(
                "Terugzetten naar de vorige beeldscherminstellingen over {n} seconde.",
                "Reverting to previous display settings in {n} second.",
            ),
            t(
                "Terugzetten naar de vorige beeldscherminstellingen over {n} seconden.",
                "Reverting to previous display settings in {n} seconds.",
            ),
        )
    };
    let text = c::label(&body(seconds), "st-countdown", true);
    let dialog = Dialog::new(t("Deze beeldscherminstellingen behouden?", "Keep these display settings?"))
        .body(&text)
        .primary(t("Wijzigingen behouden", "Keep changes"))
        .close_label(t("Terugzetten", "Revert"))
        .default_button(DialogResult::Primary);
    let n = Rc::new(Cell::new(seconds));
    let d2 = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        if !d2.is_open() {
            return glib::ControlFlow::Break;
        }
        n.set(n.get() - 1);
        text.set_label(&body(n.get().max(0)));
        if n.get() <= 0 {
            d2.close(DialogResult::Close);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    dialog.show(&p.anchor(), move |r| done(r == DialogResult::Primary));
}

struct State {
    data: RefCell<Value>,
    supported: Cell<bool>,
    sel: RefCell<String>,
    monitors_slot: gtk::Box,
    scale_slot: gtk::Box,
    tiles: RefCell<Vec<(String, gtk::Button)>>,
}

impl State {
    fn monitors(&self) -> Vec<Value> {
        self.data.borrow()["monitors"].as_array().cloned().unwrap_or_default()
    }

    fn selected(&self) -> Option<Value> {
        let mons = self.monitors();
        let sel = self.sel.borrow().clone();
        mons.iter().find(|m| m["name"].as_str() == Some(sel.as_str())).cloned().or_else(|| mons.first().cloned())
    }
}

fn focused_name(mons: &[Value]) -> String {
    mons.iter()
        .find(|m| m["focused"] == true)
        .or_else(|| mons.first())
        .and_then(|m| m["name"].as_str())
        .unwrap_or("")
        .to_owned()
}

/// A drop-down that shows one fixed value, greyed out (GNOME: read-only monitors).
fn fixed(label: &str) -> Dropdown {
    let d = Dropdown::new(vec![DropdownOption::new("cur", label)], Some("cur"));
    d.widget().set_sensitive(false);
    d
}

fn render_scale(p: &Page, st: &Rc<State>) {
    let group = SettingsGroup::new(t("Schaal en indeling", "Scale & layout"), "");
    if !st.supported.get() {
        // the host still reports the monitors read-only: show the current values greyed out (like Windows shows
        // settings it cannot change), otherwise "not available" cards
        let mons = st.monitors();
        let ro = mons.iter().find(|m| m["focused"] == true).or_else(|| mons.first()).cloned();
        group.add(common::notice(t(
            "Beeldscherminstellingen wijzigen kan hier niet. Gebruik daarvoor de app Instellingen van Ubuntu.",
            "Display settings can’t be changed here. Use Ubuntu’s Settings app instead.",
        ))
        .widget());
        match ro {
            Some(ro) => {
                let card = SettingsCard::new("zoom-in", t("Schaal", "Scale"), t("De grootte van tekst, apps en andere items wijzigen", "Change the size of text, apps and other items"));
                card.set_control(&common::sized(fixed(&format!("{}%", (num(&ro, "scale", 1.0) * 100.0).round() as i64)).widget(), 220));
                group.add(card.widget());
                let card = SettingsCard::new(
                    "display",
                    t("Beeldschermresolutie", "Display resolution"),
                    t("De resolutie aanpassen aan het aangesloten beeldscherm", "Adjust the resolution to fit your connected display"),
                );
                card.set_control(&common::sized(fixed(&format!("{} × {}", num(&ro, "width", 0.0) as i64, num(&ro, "height", 0.0) as i64)).widget(), 220));
                group.add(card.widget());
                let card = SettingsCard::new(
                    "refresh",
                    t("Vernieuwingsfrequentie", "Refresh rate"),
                    t("Een hogere frequentie geeft vloeiendere beweging, maar kost meer stroom", "A higher rate gives smoother motion but uses more power"),
                );
                card.set_control(&common::sized(fixed(&hz(num(&ro, "refresh", 60.0))).widget(), 220));
                group.add(card.widget());
            }
            None => {
                group.add(
                    common::unavailable_card("zoom-in", t("Schaal", "Scale"), Some(t("De grootte van tekst, apps en andere items wijzigen", "Change the size of text, apps and other items")))
                        .widget(),
                );
                group.add(
                    common::unavailable_card(
                        "display",
                        t("Beeldschermresolutie", "Display resolution"),
                        Some(t("De resolutie aanpassen aan het aangesloten beeldscherm", "Adjust the resolution to fit your connected display")),
                    )
                    .widget(),
                );
            }
        }
        common::set_slot(&st.scale_slot, &[group.widget().clone().upcast()]);
        return;
    }
    let Some(mon) = st.selected() else { return };
    let all = modes(&mon);
    let (mw, mh) = (num(&mon, "width", 0.0) as i64, num(&mon, "height", 0.0) as i64);
    let cur = parse_mode(&current_mode(&mon)).unwrap_or(Mode { w: mw, h: mh, hz: num(&mon, "refresh", 60.0), raw: String::new() });
    let scale = num(&mon, "scale", 1.0);

    // scale
    let mut scale_opts: Vec<f64> = SCALES.to_vec();
    if !scale_opts.iter().any(|s| (s - scale).abs() < 1e-6) {
        scale_opts.push(scale);
    }
    scale_opts.sort_by(f64::total_cmp);
    let rec_scale = if mw >= 3800 { 1.5 } else if mw >= 3000 { 1.25 } else { 1.0 };
    let key = |s: f64| format!("{s}");
    let scale_dd = Dropdown::new(
        scale_opts
            .iter()
            .map(|s| {
                let pct = format!("{}%", (s * 100.0).round() as i64);
                DropdownOption::new(&key(*s), &if (*s - rec_scale).abs() < 1e-6 { recommended(&pct) } else { pct })
            })
            .collect(),
        Some(&key(scale)),
    );
    scale_dd.widget().set_widget_name("display-scale");
    {
        let (pp, st2, mon) = (p.clone(), st.clone(), mon.clone());
        scale_dd.connect_changed(move |v| {
            let s = v.parse::<f64>().unwrap_or(1.0);
            apply_change(&pp, &st2, &mon, json!({ "scale": s }));
        });
    }

    // resolution
    let mut res: Vec<(i64, i64)> = Vec::new();
    for m in &all {
        if !res.contains(&(m.w, m.h)) {
            res.push((m.w, m.h));
        }
    }
    res.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1)));
    if !res.contains(&(cur.w, cur.h)) {
        res.insert(0, (cur.w, cur.h));
    }
    let best = res.first().copied();
    let res_dd = Dropdown::new(
        res.iter()
            .map(|(w, h)| {
                let v = format!("{w} × {h}");
                DropdownOption::new(&format!("{w}x{h}"), &if Some((*w, *h)) == best { recommended(&v) } else { v })
            })
            .collect(),
        Some(&format!("{}x{}", cur.w, cur.h)),
    );
    res_dd.widget().set_widget_name("display-resolution");
    {
        let (pp, st2, mon, all, cur) = (p.clone(), st.clone(), mon.clone(), all.clone(), cur.clone());
        res_dd.connect_changed(move |v| {
            let (w, h) = v.split_once('x').map(|(a, b)| (a.parse::<i64>().unwrap_or(0), b.parse::<i64>().unwrap_or(0))).unwrap_or((0, 0));
            let mut cands: Vec<&Mode> = all.iter().filter(|m| m.w == w && m.h == h).collect();
            cands.sort_by(|a, b| b.hz.total_cmp(&a.hz));
            let pick = cands.iter().find(|m| m.hz.round() == cur.hz.round()).or(cands.first()).map(|m| m.raw.clone());
            apply_change(&pp, &st2, &mon, json!({ "mode": pick.unwrap_or_else(|| format!("{w}x{h}")) }));
        });
    }

    // refresh rate
    let mut rates: Vec<Mode> = all.iter().filter(|m| m.w == cur.w && m.h == cur.h).cloned().collect();
    rates.sort_by(|a, b| b.hz.total_cmp(&a.hz));
    let fallback_raw = format!("{}x{}@{}", cur.w, cur.h, cur.hz);
    let rate_opts: Vec<DropdownOption> = if rates.is_empty() {
        vec![DropdownOption::new(&fallback_raw, &hz(cur.hz))]
    } else {
        rates.iter().map(|m| DropdownOption::new(&m.raw, &hz(m.hz))).collect()
    };
    let rate_sel = if rates.is_empty() { fallback_raw } else { current_mode(&mon) };
    let rate_dd = Dropdown::new(rate_opts, Some(&rate_sel));
    rate_dd.widget().set_widget_name("display-refresh");
    {
        let (pp, st2, mon) = (p.clone(), st.clone(), mon.clone());
        rate_dd.connect_changed(move |v| apply_change(&pp, &st2, &mon, json!({ "mode": v })));
    }

    let mons = st.monitors();
    let idx = mons.iter().position(|m| m["name"] == mon["name"]).map(|i| i + 1).unwrap_or(1);
    if mons.len() > 1 {
        group.set_title(&common::fill(t("Schaal en indeling · Beeldscherm {n}", "Scale & layout · Display {n}"), &[("n", &idx.to_string())]));
    }
    let card = SettingsCard::new("zoom-in", t("Schaal", "Scale"), t("De grootte van tekst, apps en andere items wijzigen", "Change the size of text, apps and other items"));
    card.set_control(&common::sized(scale_dd.widget(), dd_width()));
    group.add(card.widget());
    let card = SettingsCard::new(
        "display",
        t("Beeldschermresolutie", "Display resolution"),
        t("De resolutie aanpassen aan het aangesloten beeldscherm", "Adjust the resolution to fit your connected display"),
    );
    card.set_control(&common::sized(res_dd.widget(), dd_width()));
    group.add(card.widget());
    let card = SettingsCard::new(
        "refresh",
        t("Vernieuwingsfrequentie", "Refresh rate"),
        t("Een hogere frequentie geeft vloeiendere beweging, maar kost meer stroom", "A higher rate gives smoother motion but uses more power"),
    );
    card.set_control(&common::sized(rate_dd.widget(), dd_width()));
    group.add(card.widget());
    common::set_slot(&st.scale_slot, &[group.widget().clone().upcast()]);
}

/// display.set, then the 15 s question; Terugzetten (or no answer) sets the previous value again.
fn apply_change(p: &Page, st: &Rc<State>, mon: &Value, change: Value) {
    let before_mode = current_mode(mon);
    let before_scale = num(mon, "scale", 1.0);
    let name = mon["name"].clone();
    let (pp, st2) = (p.clone(), st.clone());
    p.spawn(async move {
        let mut args = change.clone();
        args["name"] = name.clone();
        if pp.run_with("display.set", args, None, true).await.is_none() {
            if pp.alive() {
                render_scale(&pp, &st2);
            }
            return;
        }
        reload(&pp, &st2).await;
        if !pp.alive() {
            return;
        }
        let (fill, wait) = super::later::<bool>();
        keep_changes_dialog(&pp, 15, fill);
        if wait.await {
            return;
        }
        let mut back = if change.get("scale").is_some() { json!({ "scale": before_scale }) } else { json!({ "mode": before_mode }) };
        back["name"] = name;
        let _ = pp.run_with("display.set", back, None, true).await;
        pp.toast(Toast::new(t("De vorige beeldscherminstellingen zijn teruggezet", "The previous display settings were restored")).severity(c::Severity::Info).timeout(3000));
        reload(&pp, &st2).await;
    });
}

async fn reload(p: &Page, st: &Rc<State>) {
    if let Ok(v) = p.call("display.list", json!({})).await
        && v["supported"] == true
    {
        *st.data.borrow_mut() = v;
    }
    if p.alive() {
        render_monitors(p, st);
    }
}

/// The arrangement card (display.js Arrangement): the monitors to scale, numbered; a click selects one.
fn arrangement(p: &Page, st: &Rc<State>) -> gtk::Widget {
    let fixed = gtk::Fixed::new();
    fixed.set_size_request(-1, 190);
    fixed.add_css_class("st-arrange");
    let mons = st.monitors();
    let mut tiles = Vec::new();
    for (i, m) in mons.iter().enumerate() {
        let name = m["name"].as_str().unwrap_or("").to_owned();
        let desc = m["description"].as_str().filter(|s| !s.is_empty()).unwrap_or(&name).to_owned();
        let b = gtk::Button::with_label(&(i + 1).to_string());
        b.add_css_class("st-arrange-mon");
        b.set_tooltip_text(Some(&desc));
        b.set_widget_name(&format!("display-monitor-{}", i + 1));
        let (pp, st2, n) = (p.clone(), st.clone(), name.clone());
        b.connect_clicked(move |_| {
            *st2.sel.borrow_mut() = n.clone();
            mark_tiles(&st2);
            render_scale(&pp, &st2);
        });
        fixed.put(&b, 0.0, 0.0);
        tiles.push((name, b));
    }
    *st.tiles.borrow_mut() = tiles;
    mark_tiles(st);
    let clamp = Clamp::new(&fixed, 0);
    clamp.set_hexpand(true);
    let (st2, fx) = (st.clone(), fixed.clone());
    clamp.connect_width(move |w| layout_tiles(&st2, &fx, w));
    clamp.upcast()
}

fn mark_tiles(st: &State) {
    let sel = st.sel.borrow().clone();
    for (n, b) in st.tiles.borrow().iter() {
        if *n == sel {
            b.add_css_class("w11-selected");
        } else {
            b.remove_css_class("w11-selected");
        }
    }
}

fn layout_tiles(st: &State, fixed: &gtk::Fixed, width: i32) {
    let mons = st.monitors();
    if mons.is_empty() {
        return;
    }
    let rects: Vec<(f64, f64, f64, f64)> = mons
        .iter()
        .map(|m| {
            let s = num(m, "scale", 1.0).max(0.1);
            (num(m, "x", 0.0), num(m, "y", 0.0), num(m, "width", 0.0) / s, num(m, "height", 0.0) / s)
        })
        .collect();
    let min_x = rects.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    let min_y = rects.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let max_x = rects.iter().map(|r| r.0 + r.2).fold(f64::NEG_INFINITY, f64::max);
    let max_y = rects.iter().map(|r| r.1 + r.3).fold(f64::NEG_INFINITY, f64::max);
    let (aw, ah) = (if width > 0 { width as f64 } else { 640.0 }, 190.0);
    let (sw, sh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let k = ((aw - 48.0) / sw).min((ah - 40.0) / sh);
    let (ox, oy) = ((aw - sw * k) / 2.0, (ah - sh * k) / 2.0);
    for ((_, b), r) in st.tiles.borrow().iter().zip(rects.iter()) {
        let (x, y) = (ox + (r.0 - min_x) * k + 2.0, oy + (r.1 - min_y) * k + 2.0);
        b.set_size_request(((r.2 * k) - 4.0).max(8.0) as i32, ((r.3 * k) - 4.0).max(8.0) as i32);
        fixed.move_(b, x, y);
    }
}

fn render_monitors(p: &Page, st: &Rc<State>) {
    let mons = st.monitors();
    if st.supported.get() && mons.len() > 1 {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.add_css_class("st-arrange-card");
        card.append(&arrangement(p, st));
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let hint = c::label(t("Selecteer een beeldscherm om de instellingen ervan te wijzigen.", "Select a display to change its settings."), "st-arrange-hint", true);
        hint.set_hexpand(true);
        foot.append(&hint);
        let identify = c::button(t("Identificeren", "Identify"), c::Variant::Standard);
        identify.set_widget_name("display-identify");
        let (pp, st2) = (p.clone(), st.clone());
        identify.connect_clicked(move |_| {
            let mons = st2.monitors();
            let sel = st2.sel.borrow().clone();
            let i = mons.iter().position(|m| m["name"].as_str() == Some(sel.as_str())).unwrap_or(0);
            if let Some((_, b)) = st2.tiles.borrow().get(i) {
                b.add_css_class("st-flash");
                let b = b.clone();
                widgets::after(400, move || b.remove_css_class("st-flash"));
            }
            let mon = mons.get(i).cloned().unwrap_or(Value::Null);
            let desc = mon["description"].as_str().filter(|s| !s.is_empty()).unwrap_or(&sel).to_owned();
            pp.toast(
                Toast::new(&common::fill(t("Beeldscherm {n}", "Display {n}"), &[("n", &(i + 1).to_string())]))
                    .message(&format!("{desc} · {} × {}", num(&mon, "width", 0.0) as i64, num(&mon, "height", 0.0) as i64))
                    .timeout(2500),
            );
        });
        foot.append(&identify);
        card.append(&foot);
        common::set_slot(&st.monitors_slot, &[card.upcast()]);
    } else {
        common::set_slot(&st.monitors_slot, &[]);
    }
    render_scale(p, st);
}

fn temp_to_strength(k: f64) -> f64 {
    ((6500.0 - k) / 40.0).clamp(0.0, 100.0).round()
}

fn strength_to_temp(s: f64) -> f64 {
    (6500.0 - s * 40.0).round()
}

fn display(p: Page) -> PageFuture {
    Box::pin(async move {
        let (disp, bright, night) =
            (p.call("display.list", json!({})), p.call("system.brightness.get", json!({})), p.call("system.nightLight.get", json!({})));
        let (disp, bright, night) = (disp.await, bright.await, night.await);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        // ---- monitors
        let data = disp.ok().unwrap_or(Value::Null);
        let supported = data["supported"] == true && data["monitors"].as_array().is_some_and(|a| !a.is_empty());
        let sel = if supported { focused_name(data["monitors"].as_array().map(Vec::as_slice).unwrap_or(&[])) } else { String::new() };
        let st = Rc::new(State {
            data: RefCell::new(data),
            supported: Cell::new(supported),
            sel: RefCell::new(sel),
            monitors_slot: common::slot(),
            scale_slot: common::slot(),
            tiles: RefCell::new(Vec::new()),
        });
        render_monitors(&p, &st);
        out.push(st.monitors_slot.clone().upcast());
        // a monitor plugged in or out, or changed elsewhere (host: display.changed {monitors})
        {
            let (pp, st2) = (p.clone(), st.clone());
            p.on("display.changed", move |d| {
                let Some(mons) = d["monitors"].as_array() else { return };
                if st2.data.borrow().is_null() {
                    return;
                }
                st2.data.borrow_mut()["monitors"] = json!(mons);
                let sup = st2.data.borrow()["supported"] == true && !mons.is_empty();
                st2.supported.set(sup);
                let sel = st2.sel.borrow().clone();
                if sup && !mons.iter().any(|m| m["name"].as_str() == Some(sel.as_str())) {
                    *st2.sel.borrow_mut() = focused_name(mons);
                }
                render_monitors(&pp, &st2);
            });
        }

        // ---- brightness
        let b = bright.ok().unwrap_or(Value::Null);
        let brightness_card = if b["available"] == true {
            let slider = Slider::new(0.0, 100.0, 1.0, b["value"].as_f64().unwrap_or(50.0).round());
            slider.set_value_visible(false);
            slider.widget().set_size_request(220, -1);
            slider.scale().set_widget_name("display-brightness");
            let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
            slider.connect_changed(move |v| {
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let pend = pending.clone();
                let id = glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
                    pend.borrow_mut().take();
                    kit::ctx().fire("system.brightness.set", json!({ "value": v.round() }));
                });
                *pending.borrow_mut() = Some(id);
            });
            let pp = p.clone();
            slider.connect_committed(move |v| {
                let fut = pp.run_with("system.brightness.set", json!({ "value": v.round() }), None, true);
                pp.spawn(async move {
                    let _ = fut.await;
                });
            });
            let s2 = slider.clone();
            p.on("system.brightness.changed", move |d| {
                if let Some(v) = d["value"].as_f64() {
                    s2.set_value(v.round());
                }
            });
            let card = SettingsCard::new("brightness", t("Helderheid", "Brightness"), t("De helderheid van het ingebouwde beeldscherm aanpassen", "Adjust the brightness of the built-in display"));
            card.set_control(slider.widget());
            card
        } else {
            common::unavailable_card(
                "brightness",
                t("Helderheid", "Brightness"),
                Some(t("Alleen beschikbaar voor ingebouwde schermen, zoals van een laptop", "Only available for built-in screens, such as a laptop’s")),
            )
        };

        // ---- night light
        // in the Win11 Shell session the only reason left is the missing night-light helper
        let na_text = || {
            if common::shell_info()["compositor"] == "hyprland" {
                t("Nachtlampje is op deze computer nog niet beschikbaar: er ontbreekt een onderdeel.", "Night light isn’t available on this computer yet: a component is missing.")
            } else {
                t("Nachtlampje kan hier niet worden ingesteld. Gebruik daarvoor de app Instellingen van Ubuntu.", "Night light can’t be set up here. Use Ubuntu’s Settings app instead.")
            }
        };
        let nl = night.ok();
        // system.nightLight.get().supported decides; an older host without the method keeps the toggle
        let night_supported = match &nl {
            None => true,
            Some(v) => {
                let s = if v["supported"].is_null() { &v["available"] } else { &v["supported"] };
                *s != false
            }
        };
        let night_card: gtk::Widget = if night_supported {
            let note = common::notice(na_text());
            let note_wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
            note_wrap.add_css_class("st-expander-note");
            note_wrap.append(note.widget());
            note_wrap.set_visible(false);
            let warned = Rc::new(Cell::new(false));
            let apply_night = {
                let (pp, note_wrap, warned) = (p.clone(), note_wrap.clone(), warned.clone());
                Rc::new(move |enabled: bool, temperature: f64| {
                    let (pp, note_wrap, warned) = (pp.clone(), note_wrap.clone(), warned.clone());
                    let wait = pp.call("system.nightLight.set", json!({ "enabled": enabled, "temperature": temperature }));
                    pp.clone().spawn(async move {
                        if let Err(e) = wait.await
                            && !warned.get()
                        {
                            warned.set(true);
                            if e.code == "unknown_method" || e.code == "unsupported" {
                                note_wrap.set_visible(true);
                            } else {
                                pp.error(&e, Some(t("Nachtlampje kon niet worden ingesteld", "Night light couldn’t be set")));
                            }
                        }
                    });
                })
            };
            let temp = || common::setting_f64("nightLight.temperature").unwrap_or(4500.0);
            let toggle = ToggleSwitch::new(common::setting_bool("nightLight.enabled", false));
            toggle.switch().set_widget_name("display-night-light");
            {
                let (pp, an) = (p.clone(), apply_night.clone());
                toggle.connect_toggled(move |on| {
                    common::set_setting(&pp, "nightLight.enabled", json!(on), |_| {});
                    an(on, temp());
                });
            }
            let strength = Slider::new(0.0, 100.0, 1.0, temp_to_strength(temp()));
            strength.set_value_visible(false);
            strength.widget().set_size_request(220, -1);
            {
                let (pp, an) = (p.clone(), apply_night.clone());
                strength.connect_committed(move |v| {
                    let k = strength_to_temp(v);
                    common::set_setting(&pp, "nightLight.temperature", json!(k), |_| {});
                    if common::setting_bool("nightLight.enabled", false) {
                        an(true, k);
                    }
                });
            }
            {
                let (tg, sl) = (toggle.clone(), strength.clone());
                p.on_settings(move |_, _| {
                    tg.set_active(common::setting_bool("nightLight.enabled", false));
                    sl.set_value(temp_to_strength(temp()));
                });
            }
            let exp = SettingsExpander::new("night-light", t("Nachtlampje", "Night light"), t("Warmere kleuren gebruiken om blauw licht te verminderen", "Use warmer colours to help block blue light"));
            exp.header().set_control(toggle.widget());
            let sc = SettingsCard::new("", t("Sterkte", "Strength"), t("Hoe warmer, hoe minder blauw licht", "The warmer, the less blue light"));
            sc.set_control(strength.widget());
            exp.add(sc.widget());
            exp.add(&note_wrap);
            exp.widget().clone().upcast()
        } else {
            common::unavailable_card("night-light", t("Nachtlampje", "Night light"), Some(na_text())).widget().clone()
        };

        let group = SettingsGroup::new(t("Helderheid en kleur", "Brightness & colour"), "");
        group.add(brightness_card.widget());
        group.add(&night_card);
        out.push(group.widget().clone().upcast());
        out.push(st.scale_slot.clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card_with(&p, "textsize", None, Some(t("Tekst groter maken in Win11 Shell", "Make text bigger in Win11 Shell")), None).widget());
        related.add(common::link_card(&p, "colors").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}
