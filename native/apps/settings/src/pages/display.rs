//! Instellingen › Systeem › Beeldscherm and › Geavanceerd beeldscherm (batch D, notes/claude/settings-parity.md §2,
//! amendments 19, 31, 32, 64, 69), in Windows 11's layout: the arrangement with Identificeren and the presentation
//! mode, Helderheid en kleur (brightness through mutter, night light, privacy screen, HDR, Kleurprofiel), Schaal en
//! indeling (scale with custom scaling, resolution, orientation, rotation lock, the Meerdere beeldschermen expander)
//! and Geavanceerd beeldscherm. Under GNOME every monitor setting goes through Mutter's DisplayConfig (host
//! display.state / display.apply): a change applies at once and Windows' "Deze beeldscherminstellingen behouden?"
//! asks for 15 s (extension/displayBridge.js holds back GNOME Shell's own dialog); under Hyprland the page keeps
//! display.list / display.set with the same dialog (6ebf169f:vendor/ui/js/apps/settings/pages/display.js).
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use crate::widgets::{self, Clamp};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Slider, Toast, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("display", display), ("display-advanced", advanced)];
pub const CSS: &str = "button.st-arrange-mon.st-dragging { opacity: 0.8; }\n.st-info-key { color: var(--text-2); }";

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

/// The Hyprland page (and the notice without either backend): display.list / display.set, system.nightLight.*.
fn hypr_display(p: Page) -> PageFuture {
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
            // the switch on a card of its own (an expander's header button takes the pointer's clicks)
            let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let head = SettingsCard::new("night-light", t("Nachtlampje", "Night light"), t("Warmere kleuren gebruiken om blauw licht te verminderen", "Use warmer colours to help block blue light"));
            head.set_control(toggle.widget());
            col.append(head.widget());
            let sc = SettingsCard::new("", t("Sterkte", "Strength"), t("Hoe warmer, hoe minder blauw licht", "The warmer, the less blue light"));
            sc.set_control(strength.widget());
            col.append(sc.widget());
            col.append(&note_wrap);
            col.upcast()
        } else {
            common::unavailable_card("night-light", t("Nachtlampje", "Night light"), Some(na_text())).widget().clone()
        };

        let group = SettingsGroup::new(t("Helderheid en kleur", "Brightness & colour"), "");
        group.add(brightness_card.widget());
        group.add(&night_card);
        out.push(group.widget().clone().upcast());
        out.push(st.scale_slot.clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card_with(&p, "textsize", None, Some(t("Tekst groter maken in WinPoint", "Make text bigger in WinPoint")), None).widget());
        related.add(common::link_card(&p, "colors").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// =================================================================================================== GNOME: model

/// One mode of a monitor (display.state's `modes`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GMode {
    pub id: String,
    pub w: i64,
    pub h: i64,
    pub hz: f64,
    pub scales: Vec<f64>,
    pub preferred_scale: f64,
    pub preferred: bool,
    pub variable: bool,
}

/// One monitor (display.state's `monitors`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GMonitor {
    pub connector: String,
    pub name: String,
    pub vendor: String,
    pub product: String,
    pub serial: String,
    pub builtin: bool,
    pub for_lease: bool,
    pub modes: Vec<GMode>,
    pub current: Option<String>,
    pub preferred: Option<String>,
    pub underscanning: Option<bool>,
    pub color_mode: Option<i64>,
    pub color_modes: Vec<i64>,
    pub rgb_range: Option<i64>,
    pub privacy: bool,
}

/// One logical monitor (display.state's `logical`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GLogical {
    pub x: i64,
    pub y: i64,
    pub scale: f64,
    pub transform: i64,
    pub primary: bool,
    pub monitors: Vec<String>,
}

/// display.state as the page works with it.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct GState {
    pub serial: i64,
    pub allowed: bool,
    /// layout-mode 2: logical monitors are as large as their modes (no division by the scale)
    pub physical: bool,
    pub orientation_managed: bool,
    pub monitors: Vec<GMonitor>,
    pub logical: Vec<GLogical>,
}

fn jf(v: &Value, k: &str, d: f64) -> f64 {
    v[k].as_f64().unwrap_or(d)
}

fn ji(v: &Value, k: &str) -> i64 {
    v[k].as_i64().unwrap_or(0)
}

fn js(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_owned()
}

impl GState {
    pub(crate) fn parse(v: &Value) -> GState {
        let monitors = v["monitors"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| GMonitor {
                connector: js(m, "connector"),
                name: m["displayName"].as_str().filter(|n| !n.is_empty()).unwrap_or(m["connector"].as_str().unwrap_or("")).to_owned(),
                vendor: js(m, "vendor"),
                product: js(m, "product"),
                serial: js(m, "serial"),
                builtin: m["builtin"] == true,
                for_lease: m["forLease"] == true,
                modes: m["modes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|md| GMode {
                        id: js(md, "id"),
                        w: ji(md, "width"),
                        h: ji(md, "height"),
                        hz: jf(md, "refresh", 60.0),
                        scales: md["scales"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect(),
                        preferred_scale: jf(md, "preferredScale", 1.0),
                        preferred: md["preferred"] == true,
                        variable: md["variable"] == true,
                    })
                    .collect(),
                current: m["current"].as_str().map(str::to_owned),
                preferred: m["preferred"].as_str().map(str::to_owned),
                underscanning: m["underscanning"].as_bool(),
                color_mode: m["colorMode"].as_i64(),
                color_modes: m["colorModes"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect(),
                rgb_range: m["rgbRange"].as_i64(),
                privacy: m["privacyScreen"].is_object(),
            })
            .collect();
        let logical = v["logical"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| GLogical {
                x: ji(l, "x"),
                y: ji(l, "y"),
                scale: jf(l, "scale", 1.0),
                transform: ji(l, "transform"),
                primary: l["primary"] == true,
                monitors: l["monitors"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned)).collect(),
            })
            .collect();
        GState {
            serial: ji(v, "serial"),
            allowed: v["allowed"] != false,
            physical: v["layoutMode"] == 2,
            orientation_managed: v["panelOrientationManaged"] == true,
            monitors,
            logical,
        }
    }

    pub(crate) fn monitor(&self, connector: &str) -> Option<&GMonitor> {
        self.monitors.iter().find(|m| m.connector == connector)
    }

    /// The monitors a layout can use (a monitor leased to a VR app is not).
    pub(crate) fn usable(&self) -> Vec<&GMonitor> {
        self.monitors.iter().filter(|m| !m.for_lease).collect()
    }

    /// Windows' number of a monitor (1-based, display.state's order: the labels of Identificeren).
    pub(crate) fn number(&self, connector: &str) -> usize {
        self.monitors.iter().position(|m| m.connector == connector).map(|i| i + 1).unwrap_or(1)
    }

    pub(crate) fn logical_of(&self, connector: &str) -> Option<usize> {
        self.logical.iter().position(|l| l.monitors.iter().any(|c| c == connector))
    }
}

impl GMonitor {
    pub(crate) fn mode(&self, id: &str) -> Option<&GMode> {
        self.modes.iter().find(|m| m.id == id)
    }

    /// The current mode, else the preferred one, else the first.
    pub(crate) fn active_mode(&self) -> Option<&GMode> {
        self.current
            .as_deref()
            .and_then(|id| self.mode(id))
            .or_else(|| self.preferred.as_deref().and_then(|id| self.mode(id)))
            .or_else(|| self.modes.first())
    }

    pub(crate) fn hdr_capable(&self) -> bool {
        self.color_modes.contains(&1)
    }
}

/// A monitor in a layout to apply.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MConf {
    pub connector: String,
    pub mode: String,
    pub underscanning: Option<bool>,
    pub color_mode: Option<i64>,
    pub rgb_range: Option<i64>,
}

/// A logical monitor in a layout to apply.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LConf {
    pub x: i64,
    pub y: i64,
    pub scale: f64,
    pub transform: i64,
    pub primary: bool,
    pub monitors: Vec<MConf>,
}

/// A monitor's entry with its current properties: mutter resets an underscanning or colour mode a layout leaves
/// out, so every apply carries them.
fn mconf(m: &GMonitor, mode: &str) -> MConf {
    MConf {
        connector: m.connector.clone(),
        mode: mode.to_owned(),
        underscanning: m.underscanning,
        color_mode: if m.color_modes.is_empty() { None } else { Some(m.color_mode.unwrap_or(0)) },
        rgb_range: m.rgb_range,
    }
}

/// The layout as it is now.
pub(crate) fn config_of(st: &GState) -> Vec<LConf> {
    st.logical
        .iter()
        .map(|l| LConf {
            x: l.x,
            y: l.y,
            scale: l.scale,
            transform: l.transform,
            primary: l.primary,
            monitors: l
                .monitors
                .iter()
                .filter_map(|c| st.monitor(c))
                .filter_map(|m| m.active_mode().map(|md| mconf(m, &md.id)))
                .collect(),
        })
        .filter(|l| !l.monitors.is_empty())
        .collect()
}

/// display.apply's arguments for a layout (persistent: the keep question follows).
pub(crate) fn args_of(st: &GState, cfg: &[LConf]) -> Value {
    let logical: Vec<Value> = cfg
        .iter()
        .map(|l| {
            let monitors: Vec<Value> = l
                .monitors
                .iter()
                .map(|m| {
                    let mut props = serde_json::Map::new();
                    if let Some(u) = m.underscanning {
                        props.insert("underscanning".into(), json!(u));
                    }
                    if let Some(c) = m.color_mode {
                        props.insert("color-mode".into(), json!(c));
                    }
                    if let Some(r) = m.rgb_range {
                        props.insert("rgb-range".into(), json!(r));
                    }
                    json!({"connector": m.connector, "mode": m.mode, "props": props})
                })
                .collect();
            json!({"x": l.x, "y": l.y, "scale": l.scale, "transform": l.transform, "primary": l.primary, "monitors": monitors})
        })
        .collect();
    json!({"serial": st.serial, "method": "persistent", "logical": logical})
}

/// Whether `scale` is one of `scales` (mutter's supported scales are exact fractions such as 5/3).
fn has_scale(scales: &[f64], scale: f64) -> bool {
    scales.iter().any(|s| (s - scale).abs() < 1e-4)
}

/// A logical monitor's size on the desktop: its mode turned by the transform, divided by the scale in GNOME's
/// logical layout (mutter rounds it).
pub(crate) fn size_of(st: &GState, l: &LConf) -> (i64, i64) {
    let mode = l.monitors.first().and_then(|m| st.monitor(&m.connector).and_then(|mon| mon.mode(&m.mode)));
    let (w, h) = mode.map(|m| (m.w, m.h)).unwrap_or((1920, 1080));
    let (w, h) = if l.transform % 2 == 1 { (h, w) } else { (w, h) };
    if st.physical || l.scale <= 0.0 {
        (w, h)
    } else {
        ((w as f64 / l.scale).round() as i64, (h as f64 / l.scale).round() as i64)
    }
}

type Rect = (i64, i64, i64, i64);

pub(crate) fn rects(st: &GState, cfg: &[LConf]) -> Vec<Rect> {
    cfg.iter()
        .map(|l| {
            let (w, h) = size_of(st, l);
            (l.x, l.y, w, h)
        })
        .collect()
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// The top-left corner at 0,0 (as GNOME Settings stores layouts).
pub(crate) fn normalize(pos: &mut [(i64, i64)]) {
    let min_x = pos.iter().map(|p| p.0).min().unwrap_or(0);
    let min_y = pos.iter().map(|p| p.1).min().unwrap_or(0);
    for p in pos.iter_mut() {
        p.0 -= min_x;
        p.1 -= min_y;
    }
}

/// New positions after the sizes changed (a scale, resolution or orientation): a monitor that touched another's
/// right or bottom edge keeps touching it, one aligned with another stays aligned; when that still overlaps, the
/// monitors go in one row in their order.
pub(crate) fn relayout(old: &[Rect], sizes: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let n = old.len();
    let mut pos: Vec<(i64, i64)> = old.iter().map(|r| (r.0, r.1)).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&k| (old[k].0, old[k].1));
    let v_overlap = |a: Rect, b: Rect| a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
    let h_overlap = |a: Rect, b: Rect| a.0 < b.0 + b.2 && b.0 < a.0 + a.2;
    for _ in 0..=n {
        for (rank, &k) in order.iter().enumerate() {
            let earlier = &order[..rank];
            let left = (0..n).find(|&j| j != k && old[j].0 + old[j].2 == old[k].0 && v_overlap(old[j], old[k]));
            if let Some(j) = left {
                pos[k].0 = pos[j].0 + sizes[j].0;
            } else if let Some(&j) = earlier.iter().find(|&&j| old[j].0 == old[k].0) {
                pos[k].0 = pos[j].0;
            }
            let top = (0..n).find(|&j| j != k && old[j].1 + old[j].3 == old[k].1 && h_overlap(old[j], old[k]));
            if let Some(j) = top {
                pos[k].1 = pos[j].1 + sizes[j].1;
            } else if let Some(&j) = earlier.iter().find(|&&j| old[j].1 == old[k].1) {
                pos[k].1 = pos[j].1;
            }
        }
    }
    normalize(&mut pos);
    let placed: Vec<Rect> = pos.iter().zip(sizes).map(|(p, s)| (p.0, p.1, s.0, s.1)).collect();
    let clash = (0..n).any(|a| (0..n).any(|b| a < b && overlaps(placed[a], placed[b])));
    if clash {
        let mut x = 0;
        for &k in &order {
            pos[k] = (x, 0);
            x += sizes[k].0;
        }
    }
    pos
}

/// Whether two monitors share part of an edge (mutter's `mtk_rectangle_is_adjacent_to`).
fn adjacent(a: Rect, b: Rect) -> bool {
    ((a.0 == b.0 + b.2 || a.0 + a.2 == b.0) && a.1 < b.1 + b.3 && b.1 < a.1 + a.3)
        || ((a.1 == b.1 + b.3 || a.1 + a.3 == b.1) && a.0 < b.0 + b.2 && b.0 < a.0 + a.2)
}

/// Whether every monitor is reached from the first one through shared edges: mutter's verify refuses a layout in
/// which a monitor touches none.
pub(crate) fn connected(rects: &[Rect]) -> bool {
    if rects.len() < 2 {
        return true;
    }
    let mut seen = vec![false; rects.len()];
    let mut todo = vec![0];
    seen[0] = true;
    while let Some(i) = todo.pop() {
        for (j, r) in rects.iter().enumerate() {
            if !seen[j] && adjacent(rects[i], *r) {
                seen[j] = true;
                todo.push(j);
            }
        }
    }
    seen.into_iter().all(|s| s)
}

/// Where a dragged monitor lands: next to another monitor's edge, nearest to where it was dropped (edges within
/// 24 px line up), never overlapping, and only where every monitor still touches the others (three or more: the
/// middle one dragged away would leave one alone); None when nothing fits (the monitor goes back).
pub(crate) fn snap(rects: &[Rect], moved: usize, x: i64, y: i64) -> Option<(i64, i64)> {
    let (mw, mh) = (rects[moved].2, rects[moved].3);
    let near = |a: i64, b: i64| (a - b).abs() <= 24;
    let mut best: Option<((i64, i64), i64)> = None;
    for (j, r) in rects.iter().enumerate() {
        if j == moved {
            continue;
        }
        let mut cands = Vec::new();
        let cy = {
            let mut cy = y.clamp(r.1 - mh + 1, r.1 + r.3 - 1);
            if near(cy, r.1) {
                cy = r.1;
            } else if near(cy + mh, r.1 + r.3) {
                cy = r.1 + r.3 - mh;
            }
            cy
        };
        let cx = {
            let mut cx = x.clamp(r.0 - mw + 1, r.0 + r.2 - 1);
            if near(cx, r.0) {
                cx = r.0;
            } else if near(cx + mw, r.0 + r.2) {
                cx = r.0 + r.2 - mw;
            }
            cx
        };
        cands.push((r.0 + r.2, cy));
        cands.push((r.0 - mw, cy));
        cands.push((cx, r.1 + r.3));
        cands.push((cx, r.1 - mh));
        for c in cands {
            let placed = (c.0, c.1, mw, mh);
            if rects.iter().enumerate().any(|(k, o)| k != moved && overlaps(placed, *o)) {
                continue;
            }
            let mut layout = rects.to_vec();
            layout[moved] = placed;
            if !connected(&layout) {
                continue;
            }
            let d = (c.0 - x).pow(2) + (c.1 - y).pow(2);
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((c, d));
            }
        }
    }
    best.map(|(c, _)| c)
}

/// `cfg` with new positions for new sizes.
fn placed(st: &GState, before: &[LConf], mut cfg: Vec<LConf>) -> Vec<LConf> {
    let old = rects(st, before);
    let sizes: Vec<(i64, i64)> = cfg.iter().map(|l| size_of(st, l)).collect();
    let pos = relayout(&old, &sizes);
    for (l, p) in cfg.iter_mut().zip(pos) {
        l.x = p.0;
        l.y = p.1;
    }
    cfg
}

/// The mode of `m` with this size nearest to `hz` (fixed rates unless `variable`).
fn mode_like<'a>(m: &'a GMonitor, w: i64, h: i64, hz: f64, variable: bool) -> Option<&'a GMode> {
    let same: Vec<&GMode> = m.modes.iter().filter(|md| md.w == w && md.h == h).collect();
    let pick = |v: bool| same.iter().copied().filter(|md| md.variable == v).min_by(|a, b| (a.hz - hz).abs().total_cmp(&(b.hz - hz).abs()));
    pick(variable).or_else(|| pick(!variable))
}

/// The layout with `connector` at mode `id`; in a mirror every monitor takes the same size. The scale stays when the
/// new mode supports it, else the mode's preferred scale.
pub(crate) fn with_mode(st: &GState, cfg: &[LConf], connector: &str, id: &str) -> Option<Vec<LConf>> {
    let li = cfg.iter().position(|l| l.monitors.iter().any(|m| m.connector == connector))?;
    let target = st.monitor(connector)?.mode(id)?.clone();
    let mut next = cfg.to_vec();
    let mut scales: Option<Vec<f64>> = None;
    for mc in next[li].monitors.iter_mut() {
        let mon = st.monitor(&mc.connector)?;
        let md = if mc.connector == connector { mon.mode(id)? } else { mode_like(mon, target.w, target.h, target.hz, target.variable)? };
        mc.mode = md.id.clone();
        scales = Some(match scales {
            None => md.scales.clone(),
            Some(prev) => prev.into_iter().filter(|s| has_scale(&md.scales, *s)).collect(),
        });
    }
    let scales = scales.unwrap_or_default();
    if !has_scale(&scales, next[li].scale) {
        next[li].scale = if has_scale(&scales, target.preferred_scale) { target.preferred_scale } else { scales.first().copied().unwrap_or(1.0) };
    }
    Some(placed(st, cfg, next))
}

pub(crate) fn with_scale(st: &GState, cfg: &[LConf], connector: &str, scale: f64) -> Option<Vec<LConf>> {
    let li = cfg.iter().position(|l| l.monitors.iter().any(|m| m.connector == connector))?;
    let mut next = cfg.to_vec();
    next[li].scale = scale;
    Some(placed(st, cfg, next))
}

pub(crate) fn with_transform(st: &GState, cfg: &[LConf], connector: &str, transform: i64) -> Option<Vec<LConf>> {
    let li = cfg.iter().position(|l| l.monitors.iter().any(|m| m.connector == connector))?;
    let mut next = cfg.to_vec();
    next[li].transform = transform;
    Some(placed(st, cfg, next))
}

pub(crate) fn with_primary(cfg: &[LConf], connector: &str) -> Option<Vec<LConf>> {
    let li = cfg.iter().position(|l| l.monitors.iter().any(|m| m.connector == connector))?;
    let mut next = cfg.to_vec();
    for (k, l) in next.iter_mut().enumerate() {
        l.primary = k == li;
    }
    Some(next)
}

/// The layout with one monitor's properties changed (HDR, Aanpassen voor tv).
pub(crate) fn with_props(cfg: &[LConf], connector: &str, underscanning: Option<bool>, color_mode: Option<i64>) -> Option<Vec<LConf>> {
    let mut next = cfg.to_vec();
    let mc = next.iter_mut().flat_map(|l| l.monitors.iter_mut()).find(|m| m.connector == connector)?;
    if underscanning.is_some() {
        mc.underscanning = underscanning;
    }
    if color_mode.is_some() {
        mc.color_mode = color_mode;
    }
    Some(next)
}

/// Where a dragged monitor goes (`x`, `y` in desktop pixels); None when it cannot go there.
pub(crate) fn with_position(st: &GState, cfg: &[LConf], connector: &str, x: i64, y: i64) -> Option<Vec<LConf>> {
    let li = cfg.iter().position(|l| l.monitors.iter().any(|m| m.connector == connector))?;
    let r = rects(st, cfg);
    let (nx, ny) = snap(&r, li, x, y)?;
    let mut next = cfg.to_vec();
    next[li].x = nx;
    next[li].y = ny;
    let mut pos: Vec<(i64, i64)> = next.iter().map(|l| (l.x, l.y)).collect();
    normalize(&mut pos);
    for (l, p) in next.iter_mut().zip(pos) {
        l.x = p.0;
        l.y = p.1;
    }
    Some(next)
}

/// The presentation mode now: "mirror", "extend" or "only:<connector>".
pub(crate) fn presentation(st: &GState) -> String {
    if st.logical.iter().any(|l| l.monitors.len() > 1) {
        return "mirror".into();
    }
    let active: Vec<&String> = st.logical.iter().flat_map(|l| l.monitors.iter()).collect();
    if active.len() == 1 && st.usable().len() > 1 {
        return format!("only:{}", active[0]);
    }
    "extend".into()
}

/// Deze beeldschermen dupliceren: one logical monitor with every monitor at the largest size they all have.
pub(crate) fn mirror_config(st: &GState) -> Option<Vec<LConf>> {
    let mons = st.usable();
    let first = mons.first()?;
    let mut sizes: Vec<(i64, i64)> = first.modes.iter().map(|m| (m.w, m.h)).collect();
    sizes.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
    sizes.dedup();
    let (w, h) = sizes.into_iter().find(|(w, h)| mons.iter().all(|m| m.modes.iter().any(|md| md.w == *w && md.h == *h)))?;
    let mut monitors = Vec::new();
    let mut scales: Option<Vec<f64>> = None;
    for m in &mons {
        let md = m.modes.iter().filter(|md| md.w == w && md.h == h && !md.variable).max_by(|a, b| a.hz.total_cmp(&b.hz)).or_else(|| mode_like(m, w, h, 60.0, false))?;
        scales = Some(match scales {
            None => md.scales.clone(),
            Some(prev) => prev.into_iter().filter(|s| has_scale(&md.scales, *s)).collect(),
        });
        monitors.push(mconf(m, &md.id));
    }
    let scales = scales.unwrap_or_default();
    let primary_scale = st.logical.iter().find(|l| l.primary).map(|l| l.scale).unwrap_or(1.0);
    let scale = if has_scale(&scales, primary_scale) { primary_scale } else if has_scale(&scales, 1.0) { 1.0 } else { scales.first().copied().unwrap_or(1.0) };
    Some(vec![LConf { x: 0, y: 0, scale, transform: 0, primary: true, monitors }])
}

/// Deze beeldschermen uitbreiden: one logical monitor per monitor, side by side in their order.
pub(crate) fn extend_config(st: &GState) -> Option<Vec<LConf>> {
    let mons = st.usable();
    if mons.is_empty() {
        return None;
    }
    let primary = st
        .logical
        .iter()
        .find(|l| l.primary)
        .and_then(|l| l.monitors.first().cloned())
        .unwrap_or_else(|| mons[0].connector.clone());
    let mut cfg = Vec::new();
    for m in &mons {
        let alone = st.logical_of(&m.connector).map(|k| &st.logical[k]).filter(|l| l.monitors.len() == 1);
        let md = if alone.is_some() { m.active_mode() } else { m.preferred.as_deref().and_then(|id| m.mode(id)).or_else(|| m.active_mode()) }?;
        let scale = match alone {
            Some(l) if has_scale(&md.scales, l.scale) => l.scale,
            _ if has_scale(&md.scales, md.preferred_scale) => md.preferred_scale,
            _ => 1.0,
        };
        cfg.push(LConf { x: 0, y: 0, scale, transform: alone.map(|l| l.transform).unwrap_or(0), primary: m.connector == primary, monitors: vec![mconf(m, &md.id)] });
    }
    let mut x = 0;
    for k in 0..cfg.len() {
        cfg[k].x = x;
        x += size_of(st, &cfg[k]).0;
    }
    Some(cfg)
}

/// Alleen weergeven op N: that monitor alone.
pub(crate) fn only_config(st: &GState, connector: &str) -> Option<Vec<LConf>> {
    let m = st.monitor(connector)?;
    let alone = st.logical_of(connector).map(|k| &st.logical[k]).filter(|l| l.monitors.len() == 1);
    let md = m.active_mode()?;
    let scale = match alone {
        Some(l) if has_scale(&md.scales, l.scale) => l.scale,
        _ if has_scale(&md.scales, md.preferred_scale) => md.preferred_scale,
        _ => 1.0,
    };
    Some(vec![LConf { x: 0, y: 0, scale, transform: alone.map(|l| l.transform).unwrap_or(0), primary: true, monitors: vec![mconf(m, &md.id)] }])
}

/// "100%" for a scale.
fn pct(scale: f64) -> String {
    format!("{}%", (scale * 100.0).round() as i64)
}

/// The scales the drop-down offers: Windows' 25 % steps mutter supports for the mode, the current one and the
/// recommended (preferred) one.
pub(crate) fn scale_options(md: &GMode, current: f64) -> Vec<f64> {
    let mut out: Vec<f64> = md.scales.iter().copied().filter(|s| ((s * 100.0).round() as i64) % 25 == 0).collect();
    for extra in [current, md.preferred_scale] {
        if has_scale(&md.scales, extra) && !has_scale(&out, extra) {
            out.push(extra);
        }
    }
    if !has_scale(&out, current) {
        out.push(current);
    }
    out.sort_by(f64::total_cmp);
    out
}

/// Aangepaste schaal: the supported scale nearest to a percentage (100-500).
pub(crate) fn nearest_scale(scales: &[f64], percent: f64) -> Option<f64> {
    let want = percent / 100.0;
    scales.iter().copied().min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()))
}

/// "60 Hz", "59,95 Hz" (Windows shows a whole number when the rate is one).
pub(crate) fn rate_text(hz: f64) -> String {
    let n = if (hz - hz.round()).abs() < 0.01 { kit::format::number(hz.round(), 0) } else { kit::format::number(hz, 2) };
    common::fill(t("{n} Hz", "{n} Hz"), &[("n", &n)])
}

/// The refresh rates of a monitor's current size: (mode id, rate), fixed rates only, highest first.
pub(crate) fn rate_modes(m: &GMonitor, md: &GMode) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = m.modes.iter().filter(|x| x.w == md.w && x.h == md.h && !x.variable).map(|x| (x.id.clone(), x.hz)).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.dedup_by(|a, b| (a.1 - b.1).abs() < 0.001);
    out
}

// =================================================================================================== GNOME: page

const NIGHT: GKey = gs::key("org.gnome.settings-daemon.plugins.color", "night-light-enabled");
const AMBIENT: GKey = gs::key("org.gnome.settings-daemon.plugins.power", "ambient-enabled");
const PRIVACY: GKey = gs::key("org.gnome.desktop.privacy", "privacy-screen");
const ROTATION: GKey = gs::key("org.gnome.settings-daemon.peripherals.touchscreen", "orientation-lock");
const XWAYLAND: GKey = gs::key("org.gnome.mutter.wayland", "xwayland-scaling-factor");
const FEATURES: GKey = gs::key("org.gnome.mutter", "experimental-features");

/// The keep questions that are open: display.apply's answer and the event display.confirmChange both name one, and
/// only the first of the two asks. An answered question leaves the list, so an id the bridge gives again (a bridge of
/// an older extension that numbers from 1 after a screen lock) asks again instead of being taken for a duplicate.
#[derive(Default)]
struct OpenQuestions(Vec<i64>);

impl OpenQuestions {
    /// Whether this id asks now (false: its question is already up).
    fn ask(&mut self, id: i64) -> bool {
        if self.0.contains(&id) {
            return false;
        }
        self.0.push(id);
        let len = self.0.len();
        if len > 32 {
            self.0.drain(..len - 32);
        }
        true
    }

    /// The question was answered (kept, reverted or timed out).
    fn answered(&mut self, id: i64) {
        self.0.retain(|n| *n != id);
    }
}

thread_local! {
    static ASKED: RefCell<OpenQuestions> = RefCell::new(OpenQuestions::default());
}

/// Windows' "Deze beeldscherminstellingen behouden?" for a change GNOME Shell left to Settings (amendments 32, 64,
/// 69): once per change, then display.confirm with the answer; Terugzetten, Esc or the end of the countdown give the
/// previous settings back (mutter restores them).
fn ask_keep(p: &Page, id: &Value, seconds: i64) {
    let Some(n) = id.as_i64() else { return };
    if !ASKED.with(|a| a.borrow_mut().ask(n)) {
        return;
    }
    let anchor = p.anchor();
    keep_changes_dialog(p, seconds.clamp(1, 20), move |keep| {
        ASKED.with(|a| a.borrow_mut().answered(n));
        drop(super::call("display.confirm", json!({"keep": keep, "id": n})));
        if !keep {
            let _ = Toast::new(t("De vorige beeldscherminstellingen zijn teruggezet", "The previous display settings were restored")).severity(c::Severity::Info).timeout(3000).show(&anchor);
        }
    });
}

/// Both display pages answer the keep question that GNOME Shell relays (display.confirmChange {timeout, id}).
fn follow_keep_question(p: &Page) {
    let pp = p.clone();
    p.on("display.confirmChange", move |d| ask_keep(&pp, &d["id"], d["timeout"].as_i64().unwrap_or(15)));
}

/// One showing of a GNOME display page: the state, the selected monitor and the slots that follow them.
struct GPage {
    p: Page,
    data: RefCell<GState>,
    sel: RefCell<String>,
    notice_slot: gtk::Box,
    arrange_slot: gtk::Box,
    hdr_slot: gtk::Box,
    layout_slot: gtk::Box,
    advanced_slot: gtk::Box,
    tiles: RefCell<Vec<(String, gtk::Button)>>,
    reloading: Cell<bool>,
    /// whether Meerdere beeldschermen and Beeldscherminformatie are open: every reload builds them again
    multi_open: Cell<bool>,
    info_open: Cell<bool>,
    advanced: bool,
    /// the page's GSettings items (Draaivergrendeling is built again with Schaal en indeling)
    gs: Option<Gs>,
}

impl GPage {
    fn st(&self) -> GState {
        self.data.borrow().clone()
    }

    fn selected(&self) -> Option<GMonitor> {
        let st = self.data.borrow();
        let sel = self.sel.borrow().clone();
        st.monitor(&sel).cloned().or_else(|| default_selection(&st).and_then(|c| st.monitor(&c).cloned()))
    }
}

/// The monitor a page shows first: the primary one, else the first that is on.
fn default_selection(st: &GState) -> Option<String> {
    st.logical
        .iter()
        .find(|l| l.primary)
        .and_then(|l| l.monitors.first().cloned())
        .or_else(|| st.logical.first().and_then(|l| l.monitors.first().cloned()))
        .or_else(|| st.monitors.first().map(|m| m.connector.clone()))
}

/// Apply a layout; the keep question follows (Settings' when the bridge relays it, else GNOME Shell's own).
fn apply_layout(gp: &Rc<GPage>, cfg: Option<Vec<LConf>>) {
    let st = gp.st();
    let Some(cfg) = cfg else {
        gp.p.toast(Toast::new(t("Deze indeling kan niet worden gebruikt", "This layout can’t be used")).severity(c::Severity::Error).timeout(4000));
        render_all(gp);
        return;
    };
    if cfg == config_of(&st) {
        return;
    }
    let wait = gp.p.run_with("display.apply", args_of(&st, &cfg), Some(t("Beeldscherminstellingen konden niet worden gewijzigd", "Display settings couldn’t be changed")), false);
    let gp2 = gp.clone();
    gp.p.spawn(async move {
        let answer = wait.await;
        if !gp2.p.alive() {
            return;
        }
        if let Some(v) = &answer
            && v["confirm"] == "settings"
        {
            ask_keep(&gp2.p, &v["id"], v["timeout"].as_i64().unwrap_or(15));
        }
        gnome_reload(&gp2).await;
    });
}

async fn gnome_reload(gp: &Rc<GPage>) {
    if gp.reloading.replace(true) {
        return;
    }
    let answer = gp.p.get("display.state", json!({})).await;
    gp.reloading.set(false);
    if !gp.p.alive() {
        return;
    }
    if let Some(v) = answer
        && v["supported"] == true
    {
        let st = GState::parse(&v);
        let sel = gp.sel.borrow().clone();
        if st.monitor(&sel).is_none() {
            *gp.sel.borrow_mut() = default_selection(&st).unwrap_or_default();
        }
        *gp.data.borrow_mut() = st;
    }
    render_all(gp);
}

fn render_all(gp: &Rc<GPage>) {
    render_notice(gp);
    if gp.advanced {
        render_advanced(gp);
    } else {
        render_arrangement(gp);
        render_hdr(gp);
        render_layout(gp);
    }
}

fn render_notice(gp: &Rc<GPage>) {
    if gp.data.borrow().allowed {
        common::set_slot(&gp.notice_slot, &[]);
    } else {
        let bar = common::notice(t("Beeldscherminstellingen kunnen nu niet worden gewijzigd.", "Display settings can’t be changed right now."));
        bar.widget().set_widget_name("st-display-not-allowed");
        common::set_slot(&gp.notice_slot, &[bar.widget().clone().upcast()]);
    }
}

/// Je beeldschermen opnieuw rangschikken: the monitors to scale and numbered (drag to move, click to select),
/// Identificeren and the presentation mode beside it (amendment 19); only with more than one monitor.
fn render_arrangement(gp: &Rc<GPage>) {
    let st = gp.st();
    if st.usable().len() < 2 {
        common::set_slot(&gp.arrange_slot, &[]);
        gp.tiles.borrow_mut().clear();
        return;
    }
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("st-arrange-card");
    let fixed = gtk::Fixed::new();
    fixed.set_size_request(-1, 190);
    fixed.add_css_class("st-arrange");
    let cfg = config_of(&st);
    let rs = rects(&st, &cfg);
    let mut tiles = Vec::new();
    // one tile per logical monitor; a mirror shows "1 | 2"
    for (k, l) in cfg.iter().enumerate() {
        let label = l.monitors.iter().map(|m| st.number(&m.connector).to_string()).collect::<Vec<_>>().join(" | ");
        let connector = l.monitors.first().map(|m| m.connector.clone()).unwrap_or_default();
        let b = gtk::Button::with_label(&label);
        b.add_css_class("st-arrange-mon");
        if let Some(m) = st.monitor(&connector) {
            b.set_tooltip_text(Some(&m.name));
        }
        b.set_widget_name(&format!("st-display-monitor-{}", st.number(&connector)));
        {
            let (gp2, c2) = (gp.clone(), connector.clone());
            b.connect_clicked(move |_| {
                *gp2.sel.borrow_mut() = c2.clone();
                gnome_mark_tiles(&gp2);
                render_hdr(&gp2);
                render_layout(&gp2);
            });
        }
        if st.allowed && cfg.len() > 1 {
            drag_tile(gp, &fixed, &b, k, &connector);
        }
        fixed.put(&b, 0.0, 0.0);
        tiles.push((connector, b));
    }
    *gp.tiles.borrow_mut() = tiles;
    gnome_mark_tiles(gp);
    let clamp = Clamp::new(&fixed, 0);
    clamp.set_hexpand(true);
    {
        let (gp2, fx) = (gp.clone(), fixed.clone());
        clamp.connect_width(move |w| layout_gnome_tiles(&gp2, &fx, w, &rs));
    }
    card.append(&clamp);
    let hint = c::label(t("Selecteer een beeldscherm om de instellingen ervan te wijzigen. Sleep beeldschermen om ze opnieuw te rangschikken.", "Select a display to change its settings. Drag displays to rearrange them."), "st-arrange-hint", true);
    hint.set_xalign(0.0);
    card.append(&hint);
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let identify = c::button(t("Identificeren", "Identify"), c::Variant::Standard);
    identify.set_widget_name("st-display-identify");
    {
        let gp2 = gp.clone();
        identify.connect_clicked(move |_| {
            let wait = gp2.p.call("display.identify", json!({}));
            let gp3 = gp2.clone();
            gp2.p.spawn(async move {
                if wait.await.is_err() && gp3.p.alive() {
                    // without GNOME Shell's labels: the tiles show their numbers themselves
                    for (_, b) in gp3.tiles.borrow().iter() {
                        b.add_css_class("st-flash");
                        let b = b.clone();
                        widgets::after(600, move || b.remove_css_class("st-flash"));
                    }
                }
            });
        });
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    foot.append(&identify);
    foot.append(&spacer);
    let mut options = vec![
        DropdownOption::new("mirror", t("Deze beeldschermen dupliceren", "Duplicate these displays")),
        DropdownOption::new("extend", t("Deze beeldschermen uitbreiden", "Extend these displays")),
    ];
    if mirror_config(&st).is_none() {
        options[0].disabled = true;
    }
    for m in st.usable() {
        let n = st.number(&m.connector).to_string();
        options.push(DropdownOption::new(&format!("only:{}", m.connector), &common::fill(t("Alleen weergeven op {n}", "Show only on {n}"), &[("n", &n)])));
    }
    let mode = Dropdown::new(options, Some(&presentation(&st)));
    mode.widget().set_widget_name("st-display-presentation");
    mode.widget().set_sensitive(st.allowed);
    {
        let gp2 = gp.clone();
        mode.connect_changed(move |v| {
            let st = gp2.st();
            let cfg = match v {
                "mirror" => mirror_config(&st),
                "extend" => extend_config(&st),
                other => other.strip_prefix("only:").and_then(|c| only_config(&st, c)),
            };
            apply_layout(&gp2, cfg);
        });
    }
    foot.append(&common::sized(mode.widget(), 260));
    card.append(&foot);
    common::set_slot(&gp.arrange_slot, &[card.upcast()]);
}

fn gnome_mark_tiles(gp: &GPage) {
    let sel = gp.sel.borrow().clone();
    let st = gp.data.borrow();
    for (c, b) in gp.tiles.borrow().iter() {
        let mirrored_with_sel = st.logical_of(c).is_some_and(|k| st.logical[k].monitors.iter().any(|m| *m == sel));
        if *c == sel || mirrored_with_sel {
            b.add_css_class("w11-selected");
        } else {
            b.remove_css_class("w11-selected");
        }
    }
}

thread_local! {
    /// The last arrangement geometry: (scale k, offset x, offset y, min x, min y) of the tiles in their box.
    static GEOMETRY: Cell<(f64, f64, f64, i64, i64)> = const { Cell::new((1.0, 0.0, 0.0, 0, 0)) };
}

fn layout_gnome_tiles(gp: &GPage, fixed: &gtk::Fixed, width: i32, rs: &[Rect]) {
    if rs.is_empty() {
        return;
    }
    let min_x = rs.iter().map(|r| r.0).min().unwrap_or(0);
    let min_y = rs.iter().map(|r| r.1).min().unwrap_or(0);
    let max_x = rs.iter().map(|r| r.0 + r.2).max().unwrap_or(1);
    let max_y = rs.iter().map(|r| r.1 + r.3).max().unwrap_or(1);
    let (aw, ah) = (if width > 0 { width as f64 } else { 640.0 }, 190.0);
    let (sw, sh) = (((max_x - min_x) as f64).max(1.0), ((max_y - min_y) as f64).max(1.0));
    let k = ((aw - 48.0) / sw).min((ah - 40.0) / sh);
    let (ox, oy) = ((aw - sw * k) / 2.0, (ah - sh * k) / 2.0);
    GEOMETRY.with(|g| g.set((k, ox, oy, min_x, min_y)));
    for ((_, b), r) in gp.tiles.borrow().iter().zip(rs.iter()) {
        let (x, y) = (ox + (r.0 - min_x) as f64 * k + 2.0, oy + (r.1 - min_y) as f64 * k + 2.0);
        b.set_size_request(((r.2 as f64 * k) - 4.0).max(8.0) as i32, ((r.3 as f64 * k) - 4.0).max(8.0) as i32);
        fixed.move_(b, x, y);
    }
}

/// Windows' arrangement: drag a monitor to the edge of another; it snaps there and the layout applies.
fn drag_tile(gp: &Rc<GPage>, fixed: &gtk::Fixed, b: &gtk::Button, _index: usize, connector: &str) {
    let drag = gtk::GestureDrag::new();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let start: Rc<Cell<(f64, f64)>> = Rc::new(Cell::new((0.0, 0.0)));
    let moved = Rc::new(Cell::new(false));
    {
        let (fx, b2, start) = (fixed.clone(), b.clone(), start.clone());
        let moved = moved.clone();
        drag.connect_drag_begin(move |_, _, _| {
            let (x, y) = fx.child_position(&b2);
            start.set((x, y));
            moved.set(false);
        });
    }
    {
        let (fx, b2, start, moved) = (fixed.clone(), b.clone(), start.clone(), moved.clone());
        drag.connect_drag_update(move |g, dx, dy| {
            if !moved.get() && dx.hypot(dy) < 6.0 {
                return;
            }
            if !moved.replace(true) {
                g.set_state(gtk::EventSequenceState::Claimed);
                b2.add_css_class("st-dragging");
            }
            let (x0, y0) = start.get();
            fx.move_(&b2, x0 + dx, y0 + dy);
        });
    }
    {
        let (gp2, fx, b2, c2) = (gp.clone(), fixed.clone(), b.clone(), connector.to_owned());
        drag.connect_drag_end(move |_, _, _| {
            b2.remove_css_class("st-dragging");
            if !moved.replace(false) {
                return;
            }
            let (x, y) = fx.child_position(&b2);
            let (k, ox, oy, min_x, min_y) = GEOMETRY.with(|g| g.get());
            let lx = ((x - 2.0 - ox) / k).round() as i64 + min_x;
            let ly = ((y - 2.0 - oy) / k).round() as i64 + min_y;
            let st = gp2.st();
            let cfg = config_of(&st);
            match with_position(&st, &cfg, &c2, lx, ly) {
                Some(next) => apply_layout(&gp2, Some(next)),
                None => render_arrangement(&gp2),
            }
        });
    }
    b.add_controller(drag);
}

/// HDR gebruiken in Helderheid en kleur, for the selected monitor when it supports BT.2100.
fn render_hdr(gp: &Rc<GPage>) {
    let Some(m) = gp.selected() else {
        common::set_slot(&gp.hdr_slot, &[]);
        return;
    };
    if !m.hdr_capable() || gp.data.borrow().logical_of(&m.connector).is_none() {
        common::set_slot(&gp.hdr_slot, &[]);
        return;
    }
    let st = gp.st();
    let card = SettingsCard::new("sun", t("HDR gebruiken", "Use HDR"), t("Helderder en levendiger beeld in HDR-video’s, games en apps", "Brighter and more vibrant picture in HDR videos, games and apps"));
    let sw = ToggleSwitch::new(m.color_mode == Some(1));
    sw.switch().set_widget_name("st-display-hdr");
    sw.set_sensitive(st.allowed);
    {
        let (gp2, c2) = (gp.clone(), m.connector.clone());
        sw.connect_toggled(move |on| {
            let st = gp2.st();
            apply_layout(&gp2, with_props(&config_of(&st), &c2, None, Some(if on { 1 } else { 0 })));
        });
    }
    card.set_control(sw.widget());
    common::set_slot(&gp.hdr_slot, &[card.widget().clone()]);
}

/// Schaal en indeling (+ Meerdere beeldschermen) for the selected monitor.
fn render_layout(gp: &Rc<GPage>) {
    let st = gp.st();
    let Some(m) = gp.selected() else {
        common::set_slot(&gp.layout_slot, &[]);
        return;
    };
    let group = SettingsGroup::new(t("Schaal en indeling", "Scale & layout"), "");
    if st.usable().len() > 1 {
        group.set_title(&common::fill(t("Schaal en indeling · Beeldscherm {n}", "Scale & layout · Display {n}"), &[("n", &st.number(&m.connector).to_string())]));
    }
    let cfg = config_of(&st);
    let lconf = st.logical_of(&m.connector).and_then(|k| st.logical.get(k)).cloned();
    match (lconf, m.active_mode().cloned()) {
        (Some(l), Some(md)) => {
            scale_rows(gp, &group, &st, &m, &md, &l, &cfg);
            resolution_row(gp, &group, &st, &m, &md, &cfg);
            orientation_row(gp, &group, &st, &m, &l, &cfg);
        }
        _ => {
            // a monitor that is off (Alleen weergeven op ...): Windows offers only the presentation mode
            group.add(common::notice(t("Dit beeldscherm is uitgeschakeld. Kies hierboven waar je beeld wilt weergeven.", "This display is turned off. Choose above where to show your screen.")).widget());
        }
    }
    if common::caps()["accelerometer"] == true
        && let Some(gs) = &gp.gs
        && let Some(card) = gs.toggle_card(&ROTATION, "lock", t("Draaivergrendeling", "Rotation lock"), "", false)
    {
        group.add(card.widget());
    }
    group.add(multiple_displays(gp, &st, &m, &cfg).widget());
    common::set_slot(&gp.layout_slot, &[group.widget().clone().upcast()]);
}

fn scale_rows(gp: &Rc<GPage>, group: &SettingsGroup, st: &GState, m: &GMonitor, md: &GMode, l: &GLogical, cfg: &[LConf]) {
    let options = scale_options(md, l.scale);
    let dd = Dropdown::new(
        options
            .iter()
            .map(|s| {
                let label = if (s - md.preferred_scale).abs() < 1e-4 { recommended(&pct(*s)) } else { pct(*s) };
                DropdownOption::new(&format!("{s}"), &label)
            })
            .collect(),
        Some(&format!("{}", options.iter().copied().find(|s| (s - l.scale).abs() < 1e-4).unwrap_or(l.scale))),
    );
    dd.widget().set_widget_name("st-display-scale");
    dd.widget().set_sensitive(st.allowed);
    {
        let (gp2, c2) = (gp.clone(), m.connector.clone());
        dd.connect_changed(move |v| {
            let Ok(scale) = v.parse::<f64>() else { return };
            let st = gp2.st();
            apply_layout(&gp2, with_scale(&st, &config_of(&st), &c2, scale));
        });
    }
    // a plain card: a drop-down inside an expander's header button never gets the pointer's click (kit cards.rs, the
    // button takes it); Aangepaste schaal is the row below
    let scale_card = SettingsCard::new("zoom-in", t("Schaal", "Scale"), t("De grootte van tekst, apps en andere items wijzigen", "Change the size of text, apps and other items"));
    scale_card.set_control(&common::sized(dd.widget(), dd_width()));
    // Aangepaste schaal (Windows: 100-500 %): the nearest scale mutter supports for this mode
    let custom = sub_row(SettingsCard::new("", t("Aangepaste schaal", "Custom scaling"), t("Voer een aangepaste schaalgrootte in tussen 100% en 500%", "Enter a custom scaling size between 100% and 500%")));
    let number = c::number_box(100.0, 500.0, 1.0, (l.scale * 100.0).round(), 0);
    number.set_widget_name("st-display-custom-scale");
    let apply = c::button(t("Toepassen", "Apply"), c::Variant::Standard);
    apply.set_widget_name("st-display-custom-scale-apply");
    apply.set_sensitive(st.allowed);
    {
        let (gp2, c2, scales, nb) = (gp.clone(), m.connector.clone(), md.scales.clone(), number.clone());
        apply.connect_clicked(move |_| {
            let Some(scale) = nearest_scale(&scales, nb.value()) else { return };
            nb.set_value((scale * 100.0).round());
            let st = gp2.st();
            apply_layout(&gp2, with_scale(&st, &config_of(&st), &c2, scale));
        });
    }
    custom.set_control(&number);
    custom.add_control(&apply);
    group.add(scale_card.widget());
    group.add(custom.widget());
    let _ = cfg;
}

fn resolution_row(gp: &Rc<GPage>, group: &SettingsGroup, st: &GState, m: &GMonitor, md: &GMode, _cfg: &[LConf]) {
    let mut sizes: Vec<(i64, i64)> = m.modes.iter().map(|x| (x.w, x.h)).collect();
    sizes.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
    sizes.dedup();
    let best = m.preferred.as_deref().and_then(|id| m.mode(id)).map(|x| (x.w, x.h));
    let dd = Dropdown::new(
        sizes
            .iter()
            .map(|(w, h)| {
                let v = format!("{w} × {h}");
                DropdownOption::new(&format!("{w}x{h}"), &if Some((*w, *h)) == best { recommended(&v) } else { v })
            })
            .collect(),
        Some(&format!("{}x{}", md.w, md.h)),
    );
    dd.widget().set_widget_name("st-display-resolution");
    dd.widget().set_sensitive(st.allowed);
    {
        let (gp2, c2, hz, variable) = (gp.clone(), m.connector.clone(), md.hz, md.variable);
        dd.connect_changed(move |v| {
            let Some((w, h)) = v.split_once('x').and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?))) else { return };
            let st = gp2.st();
            let Some(mon) = st.monitor(&c2) else { return };
            let Some(id) = mode_like(mon, w, h, hz, variable).map(|x| x.id.clone()) else { return };
            apply_layout(&gp2, with_mode(&st, &config_of(&st), &c2, &id));
        });
    }
    let card = SettingsCard::new("display", t("Beeldschermresolutie", "Display resolution"), t("De resolutie aanpassen aan het aangesloten beeldscherm", "Adjust the resolution to fit your connected display"));
    card.set_control(&common::sized(dd.widget(), dd_width()));
    group.add(card.widget());
}

fn orientation_row(gp: &Rc<GPage>, group: &SettingsGroup, st: &GState, m: &GMonitor, l: &GLogical, _cfg: &[LConf]) {
    let dd = Dropdown::new(
        vec![
            DropdownOption::new("0", t("Liggend", "Landscape")),
            DropdownOption::new("1", t("Staand", "Portrait")),
            DropdownOption::new("2", t("Liggend (gespiegeld)", "Landscape (flipped)")),
            DropdownOption::new("3", t("Staand (gespiegeld)", "Portrait (flipped)")),
        ],
        Some(&(l.transform % 4).to_string()),
    );
    dd.widget().set_widget_name("st-display-orientation");
    let managed = st.orientation_managed && m.builtin;
    dd.widget().set_sensitive(st.allowed && !managed);
    if managed {
        dd.widget().set_tooltip_text(Some(t("De afdrukstand volgt de stand van het apparaat.", "The orientation follows how the device is held.")));
    }
    {
        let (gp2, c2) = (gp.clone(), m.connector.clone());
        dd.connect_changed(move |v| {
            let Ok(tr) = v.parse::<i64>() else { return };
            let st = gp2.st();
            apply_layout(&gp2, with_transform(&st, &config_of(&st), &c2, tr));
        });
    }
    let card = SettingsCard::new("restart", t("Afdrukstand van beeldscherm", "Display orientation"), "");
    card.set_control(&common::sized(dd.widget(), dd_width()));
    group.add(card.widget());
}

/// Meerdere beeldschermen (amendment 19): Dit mijn hoofdbeeldscherm maken and Ander beeldscherm detecteren.
fn multiple_displays(gp: &Rc<GPage>, st: &GState, m: &GMonitor, cfg: &[LConf]) -> SettingsExpander {
    let exp = SettingsExpander::new("devices", t("Meerdere beeldschermen", "Multiple displays"), t("Kies de presentatiemodus voor je beeldschermen", "Choose the presentation mode for your displays"));
    exp.header_button().set_widget_name("st-display-multiple");
    exp.set_expanded(gp.multi_open.get());
    {
        let gp2 = Rc::downgrade(gp);
        exp.connect_toggled(move |open| {
            if let Some(gp) = gp2.upgrade() {
                gp.multi_open.set(open);
            }
        });
    }
    let primary = st.logical_of(&m.connector).is_some_and(|k| st.logical[k].primary);
    let main = c::checkbox(t("Dit mijn hoofdbeeldscherm maken", "Make this my main display"));
    main.set_widget_name("st-display-primary");
    main.set_active(primary);
    main.set_sensitive(st.allowed && cfg.len() > 1 && !primary && st.logical_of(&m.connector).is_some());
    {
        let (gp2, c2) = (gp.clone(), m.connector.clone());
        main.connect_toggled(move |b| {
            if b.is_active() {
                let st = gp2.st();
                apply_layout(&gp2, with_primary(&config_of(&st), &c2));
            }
        });
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.append(&main);
    exp.add(&row);
    let detect_card = SettingsCard::new("", t("Ander beeldscherm detecteren", "Detect other display"), "");
    let detect = c::button(t("Detecteren", "Detect"), c::Variant::Standard);
    detect.set_widget_name("st-display-detect");
    {
        let gp2 = gp.clone();
        detect.connect_clicked(move |_| {
            let before = gp2.data.borrow().monitors.len();
            let gp3 = gp2.clone();
            gp2.p.spawn(async move {
                gnome_reload(&gp3).await;
                if gp3.p.alive() && gp3.data.borrow().monitors.len() == before {
                    gp3.p.toast(Toast::new(t("Geen ander beeldscherm gevonden", "Didn’t detect another display")).severity(c::Severity::Info).timeout(3000));
                }
            });
        });
    }
    detect_card.set_control(&detect);
    exp.add(detect_card.widget());
    exp
}

/// Helderheid (through mutter's backlight, amendment 31) with, when there is a light sensor, Windows' "Helderheid
/// automatisch wijzigen wanneer de verlichting verandert" (gsd-power's ambient-enabled) inside the expander.
fn brightness_row(p: &Page, gs: &Gs, b: &Value) -> gtk::Widget {
    if b["available"] != true {
        return common::unavailable_card(
            "brightness",
            t("Helderheid", "Brightness"),
            Some(t("Alleen beschikbaar voor ingebouwde schermen, zoals van een laptop", "Only available for built-in screens, such as a laptop’s")),
        )
        .widget()
        .clone();
    }
    let slider = Slider::new(0.0, 100.0, 1.0, b["value"].as_f64().unwrap_or(50.0).round());
    slider.set_value_visible(false);
    slider.widget().set_size_request(220, -1);
    slider.scale().set_widget_name("st-display-brightness");
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
    {
        let pp = p.clone();
        slider.connect_committed(move |v| {
            let fut = pp.run_with("system.brightness.set", json!({ "value": v.round() }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
    }
    {
        let s2 = slider.clone();
        p.on("system.brightness.changed", move |d| {
            if let Some(v) = d["value"].as_f64() {
                s2.set_value(v.round());
            }
        });
    }
    let title = t("Helderheid", "Brightness");
    let desc = t("De helderheid van het ingebouwde beeldscherm aanpassen", "Adjust the brightness of the built-in display");
    if common::caps()["ambientLight"] == true && gs.available(&AMBIENT) {
        // the slider on a card of its own, not in an expander's header button (which takes the pointer's clicks)
        let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let card = SettingsCard::new("brightness", title, desc);
        card.set_control(slider.widget());
        col.append(card.widget());
        let cb = gs.checkbox(&AMBIENT, t("Helderheid automatisch wijzigen wanneer de verlichting verandert", "Change brightness automatically when lighting changes"), false);
        let row = SettingsCard::new("", "", "");
        row.set_icon_widget(&cb);
        col.append(row.widget());
        return col.upcast();
    }
    let card = SettingsCard::new("brightness", title, desc);
    card.set_control(slider.widget());
    card.widget().clone()
}

/// Nachtlampje: GNOME's night-light-enabled switch and a chevron to its page (Windows' row).
fn night_light_row(p: &Page, gs: &Gs, status: &Value) -> Option<gtk::Widget> {
    if !gs.available(&NIGHT) {
        return None;
    }
    // a plain card with the switch and a chevron button of its own: a switch inside a clickable card's button never
    // gets the pointer's click
    let card = SettingsCard::new("night-light", t("Nachtlampje", "Night light"), t("Warmere kleuren gebruiken om blauw licht te verminderen", "Use warmer colours to help block blue light"));
    if status["supported"] == false {
        card.set_description(t("Nachtlampje is op deze computer niet beschikbaar.", "Night light isn’t available on this computer."));
    } else {
        card.set_control(gs.toggle(&NIGHT, false).widget());
    }
    let open = c::icon_button("chevron-right", t("Nachtlampje", "Night light"), 12);
    open.set_widget_name("link-nightlight");
    let pp = p.clone();
    open.connect_clicked(move |_| pp.go("nightlight"));
    card.add_control(&open);
    Some(card.widget().clone())
}

/// The GNOME Beeldscherm page.
async fn gnome_display(p: Page, state: Value) -> Content {
    let (bright, night) = (p.call("system.brightness.get", json!({})), p.call("display.nightLight.status", json!({})));
    let gs = Gs::load(&p, &[NIGHT, AMBIENT, PRIVACY, ROTATION]).await;
    let (bright, night) = (bright.await.unwrap_or(Value::Null), night.await.unwrap_or(Value::Null));
    if !p.alive() {
        return Content::new();
    }
    let st = GState::parse(&state);
    let sel = default_selection(&st).unwrap_or_default();
    let privacy_hw = st.monitors.iter().any(|m| m.privacy) || common::caps()["privacyScreen"] == true;
    let gp = Rc::new(GPage {
        p: p.clone(),
        data: RefCell::new(st),
        sel: RefCell::new(sel),
        notice_slot: common::slot(),
        arrange_slot: common::slot(),
        hdr_slot: common::slot(),
        layout_slot: common::slot(),
        advanced_slot: common::slot(),
        tiles: RefCell::new(Vec::new()),
        reloading: Cell::new(false),
        multi_open: Cell::new(false),
        info_open: Cell::new(true),
        advanced: false,
        gs: Some(gs.clone()),
    });
    let mut out: Content = vec![gp.notice_slot.clone().upcast(), gp.arrange_slot.clone().upcast()];
    // Helderheid en kleur
    let group = SettingsGroup::new(t("Helderheid en kleur", "Brightness & colour"), "");
    group.add(&brightness_row(&p, &gs, &bright));
    if let Some(row) = night_light_row(&p, &gs, &night) {
        group.add(&row);
    }
    if privacy_hw
        && let Some(card) = gs.toggle_card(&PRIVACY, "eye-off", t("Privacyscherm", "Privacy screen"), t("Kijkhoek beperken", "Restrict viewing angle"), false)
    {
        group.add(card.widget());
    }
    group.add(&gp.hdr_slot);
    // Kleurprofiel is a row here since 24H2 (amendment 19)
    group.add(common::link_card(&p, "colorprofile").widget());
    out.push(group.widget().clone().upcast());
    out.push(gp.layout_slot.clone().upcast());
    let related = SettingsGroup::new(ct::related(), "");
    related.add(common::link_card(&p, "display-advanced").widget());
    out.push(related.widget().clone().upcast());
    render_all(&gp);
    follow_changes(&gp);
    follow_keep_question(&p);
    out
}

/// display.changed (a monitor plugged in or out, a layout applied anywhere, ApplyMonitorsConfigAllowed): read the
/// state again, once per burst.
fn follow_changes(gp: &Rc<GPage>) {
    let gp2 = gp.clone();
    gp.p.on("display.changed", move |_| {
        let gp3 = gp2.clone();
        gp2.p.timeout(150, move || {
            let gp4 = gp3.clone();
            gp3.p.spawn(async move { gnome_reload(&gp4).await });
        });
    });
}

fn display(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.get("display.state", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if state["supported"] == true && state["backend"] == "mutter" {
            return gnome_display(p, state).await;
        }
        hypr_display(p).await
    })
}

// ========================================================================================= Geavanceerd beeldscherm

/// "Beeldscherm 1: Ingebouwd beeldscherm" (Windows' list of displays).
fn display_label(st: &GState, m: &GMonitor) -> String {
    common::fill(t("Beeldscherm {n}: {name}", "Display {n}: {name}"), &[("n", &st.number(&m.connector).to_string()), ("name", &m.name)])
}

/// Beeldscherminformatie's rows: (label, value).
pub(crate) fn info_rows(st: &GState, m: &GMonitor) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let lconf = config_of(st).into_iter().find(|l| l.monitors.iter().any(|x| x.connector == m.connector));
    if let (Some(l), Some(md)) = (lconf, m.active_mode()) {
        let (w, h) = size_of(st, &l);
        rows.push((t("Bureaubladmodus", "Desktop mode").to_owned(), format!("{w} × {h}, {}", rate_text(md.hz))));
        rows.push((t("Actieve signaalmodus", "Active signal mode").to_owned(), format!("{} × {}, {}", md.w, md.h, rate_text(md.hz))));
        rows.push((t("Schaal", "Scale").to_owned(), pct(l.scale)));
    } else {
        rows.push((t("Status", "Status").to_owned(), t("Uitgeschakeld", "Turned off").to_owned()));
    }
    if !m.vendor.is_empty() {
        rows.push((t("Fabrikant", "Manufacturer").to_owned(), m.vendor.clone()));
    }
    if !m.product.is_empty() {
        rows.push((t("Model", "Model").to_owned(), m.product.clone()));
    }
    if !m.serial.is_empty() && m.serial.trim_start_matches("0x").trim_start_matches('0').len() > 0 {
        rows.push((t("Serienummer", "Serial number").to_owned(), m.serial.clone()));
    }
    let space = if m.color_mode == Some(1) { t("Hoog dynamisch bereik (HDR)", "High dynamic range (HDR)") } else { t("Standaard dynamisch bereik (SDR)", "Standard dynamic range (SDR)") };
    rows.push((t("Kleurruimte", "Colour space").to_owned(), space.to_owned()));
    rows
}

fn render_advanced(gp: &Rc<GPage>) {
    let st = gp.st();
    let Some(m) = gp.selected() else {
        common::set_slot(&gp.advanced_slot, &[common::notice(ct::unavailable_here()).widget().clone().upcast()]);
        return;
    };
    let mut blocks: Vec<gtk::Widget> = Vec::new();
    // Selecteer een beeldscherm
    let pick = Dropdown::new(st.monitors.iter().map(|x| DropdownOption::new(&x.connector, &display_label(&st, x))).collect(), Some(&m.connector));
    pick.widget().set_widget_name("st-display-advanced-select");
    {
        let gp2 = gp.clone();
        pick.connect_changed(move |v| {
            *gp2.sel.borrow_mut() = v.to_owned();
            render_advanced(&gp2);
        });
    }
    let card = SettingsCard::new("display", t("Selecteer een beeldscherm om de instellingen ervan te bekijken of te wijzigen", "Select a display to view or change its settings"), "");
    card.set_control(&common::sized(pick.widget(), 280));
    let top = SettingsGroup::new("", "");
    top.add(card.widget());
    blocks.push(top.widget().clone().upcast());
    // Beeldscherminformatie
    let info = SettingsGroup::new(t("Beeldscherminformatie", "Display information"), "");
    let exp = SettingsExpander::new("info", &m.name, &common::fill(t("Beeldscherm {n}: verbonden met {connector}", "Display {n}: connected to {connector}"), &[("n", &st.number(&m.connector).to_string()), ("connector", &m.connector)]));
    exp.header_button().set_widget_name("st-display-advanced-info");
    exp.set_expanded(gp.info_open.get());
    {
        let gp2 = Rc::downgrade(gp);
        exp.connect_toggled(move |open| {
            if let Some(gp) = gp2.upgrade() {
                gp.info_open.set(open);
            }
        });
    }
    let rows = info_rows(&st, &m);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(6);
    grid.add_css_class("st-info-grid");
    for (k, (label, value)) in rows.iter().enumerate() {
        let l = c::label(label, "st-info-key", false);
        l.set_xalign(0.0);
        let v = c::label(value, "w11-body", false);
        v.set_xalign(0.0);
        v.set_selectable(true);
        grid.attach(&l, 0, k as i32, 1, 1);
        grid.attach(&v, 1, k as i32, 1, 1);
    }
    exp.add(&grid);
    let copy = c::button(t("Kopiëren", "Copy"), c::Variant::Standard);
    copy.set_widget_name("st-display-advanced-copy");
    {
        let (gp2, text) = (gp.clone(), {
            let mut s = format!("{}\n{}\n", m.name, display_label(&st, &m));
            for (label, value) in &rows {
                s.push_str(&format!("{label}: {value}\n"));
            }
            s
        });
        copy.connect_clicked(move |_| {
            let ok = common::copy_text(&text);
            gp2.p.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).timeout(2500));
        });
    }
    let copy_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    copy_row.append(&copy);
    exp.add(&copy_row);
    info.add(exp.widget());
    blocks.push(info.widget().clone().upcast());
    // Een vernieuwingsfrequentie kiezen, Dynamische vernieuwingsfrequentie, Aanpassen voor tv
    let rates = SettingsGroup::new("", "");
    let mut has_rates = false;
    if let Some(md) = m.active_mode().cloned()
        && st.logical_of(&m.connector).is_some()
    {
        let list = rate_modes(&m, &md);
        let current = if md.variable { list.iter().min_by(|a, b| (a.1 - md.hz).abs().total_cmp(&(b.1 - md.hz).abs())).map(|x| x.0.clone()) } else { Some(md.id.clone()) };
        let dd = Dropdown::new(list.iter().map(|(id, hz)| DropdownOption::new(id, &rate_text(*hz))).collect(), current.as_deref());
        dd.widget().set_widget_name("st-display-advanced-refresh");
        dd.widget().set_sensitive(st.allowed && list.len() > 1);
        {
            let (gp2, c2) = (gp.clone(), m.connector.clone());
            dd.connect_changed(move |id| {
                let st = gp2.st();
                apply_layout(&gp2, with_mode(&st, &config_of(&st), &c2, id));
            });
        }
        let card = SettingsCard::new("refresh", t("Een vernieuwingsfrequentie kiezen", "Choose a refresh rate"), t("Een hogere frequentie geeft vloeiendere beweging, maar kost meer stroom", "A higher rate gives smoother motion but uses more power"));
        card.set_control(&common::sized(dd.widget(), 200));
        rates.add(card.widget());
        has_rates = true;
        // VRR: the size's variable mode (mutter's refresh-rate-mode variable)
        let vrr = m.modes.iter().filter(|x| x.w == md.w && x.h == md.h && x.variable).max_by(|a, b| a.hz.total_cmp(&b.hz)).cloned();
        if let Some(vrr) = vrr {
            let card = SettingsCard::new("", t("Dynamische vernieuwingsfrequentie", "Dynamic refresh rate"), t("Om energie te besparen past WinPoint de vernieuwingsfrequentie automatisch aan, tot de frequentie hierboven", "To help save power, WinPoint adjusts the refresh rate up to the rate above"));
            let sw = ToggleSwitch::new(md.variable);
            sw.switch().set_widget_name("st-display-advanced-vrr");
            sw.set_sensitive(st.allowed);
            {
                let (gp2, c2, fixed_id) = (gp.clone(), m.connector.clone(), current.clone());
                let vrr_id = vrr.id.clone();
                sw.connect_toggled(move |on| {
                    let st = gp2.st();
                    let id = if on { Some(vrr_id.clone()) } else { fixed_id.clone() };
                    if let Some(id) = id {
                        apply_layout(&gp2, with_mode(&st, &config_of(&st), &c2, &id));
                    }
                });
            }
            card.set_control(sw.widget());
            rates.add(card.widget());
        }
    }
    if let Some(on) = m.underscanning
        && st.logical_of(&m.connector).is_some()
    {
        let card = SettingsCard::new("display", t("Aanpassen voor tv", "Adjust for TV"), t("Het beeld verkleinen zodat de randen op een tv zichtbaar zijn", "Shrink the picture so its edges show on a TV"));
        let sw = ToggleSwitch::new(on);
        sw.switch().set_widget_name("st-display-advanced-underscanning");
        sw.set_sensitive(st.allowed);
        {
            let (gp2, c2) = (gp.clone(), m.connector.clone());
            sw.connect_toggled(move |on| {
                let st = gp2.st();
                apply_layout(&gp2, with_props(&config_of(&st), &c2, Some(on), None));
            });
        }
        card.set_control(sw.widget());
        rates.add(card.widget());
        has_rates = true;
    }
    if has_rates {
        blocks.push(rates.widget().clone().upcast());
    }
    common::set_slot(&gp.advanced_slot, &blocks);
}

/// Schaal voor apps herstellen (GNOME's "Built-in Scaling for Legacy Apps"): on = Xwayland apps scale themselves
/// (xwayland-scaling-factor 0, automatic), off = 1 with GNOME scaling them; the mapping is GNOME Settings 50's own
/// (cc-display-panel.c's g_settings_bind_with_mapping, read from the installed binary). Shown while mutter's
/// xwayland-native-scaling feature is on, without which the key does nothing.
fn legacy_scaling_card(p: &Page, gs: &Gs) -> Option<SettingsCard> {
    let native = gs.value(&FEATURES).as_array().is_some_and(|a| a.iter().any(|f| f == "xwayland-native-scaling"));
    if !gs.available(&XWAYLAND) || !native {
        return None;
    }
    let on = |v: &Value| v.as_f64().is_some_and(|f| f.abs() < 1.2e-7);
    let card = SettingsCard::new("sparkle", t("Schaal voor apps herstellen", "Fix scaling for apps"), t("Oudere apps die schalen ondersteunen worden scherp weergegeven, maar sommige worden mogelijk niet geschaald", "Legacy apps that support scaling look crisp, but some might not scale at all"));
    let sw = ToggleSwitch::new(on(&gs.value(&XWAYLAND)));
    sw.switch().set_widget_name(&gs::control_name(p.id(), &[XWAYLAND, FEATURES], &XWAYLAND));
    if !gs.writable(&XWAYLAND) {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (gs2, sw2, pp) = (gs.clone(), sw.clone(), p.clone());
        sw.connect_toggled(move |active| {
            let (gs3, sw3) = (gs2.clone(), sw2.clone());
            gs2.set(&XWAYLAND, json!(if active { 0.0 } else { 1.0 }), move || sw3.set_active(on(&gs3.value(&XWAYLAND))));
            pp.toast(Toast::new(t("Start apps opnieuw om de nieuwe schaal toe te passen", "Restart apps for the new scaling to take effect")).severity(c::Severity::Info).timeout(4000));
        });
    }
    {
        let sw2 = sw.clone();
        gs.follow(&XWAYLAND, move |v| sw2.set_active(on(v)));
    }
    card.set_control(sw.widget());
    Some(card)
}

/// Geavanceerd beeldscherm (GNOME only, registry needs "gnome").
fn advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.get("display.state", json!({})).await.unwrap_or(Value::Null);
        let gs = Gs::load(&p, &[XWAYLAND, FEATURES]).await;
        if !p.alive() {
            return Content::new();
        }
        if state["supported"] != true {
            return vec![common::notice(t("Beeldscherminstellingen zijn hier niet beschikbaar.", "Display settings aren’t available here.")).widget().clone().upcast()];
        }
        let st = GState::parse(&state);
        let sel = p.param("display").filter(|c| st.monitor(c).is_some()).or_else(|| default_selection(&st)).unwrap_or_default();
        let gp = Rc::new(GPage {
            p: p.clone(),
            data: RefCell::new(st),
            sel: RefCell::new(sel),
            notice_slot: common::slot(),
            arrange_slot: common::slot(),
            hdr_slot: common::slot(),
            layout_slot: common::slot(),
            advanced_slot: common::slot(),
            tiles: RefCell::new(Vec::new()),
            reloading: Cell::new(false),
            multi_open: Cell::new(false),
            info_open: Cell::new(true),
            advanced: true,
            gs: None,
        });
        let mut out: Content = vec![gp.notice_slot.clone().upcast(), gp.advanced_slot.clone().upcast()];
        if let Some(card) = legacy_scaling_card(&p, &gs) {
            let group = SettingsGroup::new(t("Schaal", "Scale"), "");
            group.add(card.widget());
            out.push(group.widget().clone().upcast());
        }
        render_all(&gp);
        follow_changes(&gp);
        follow_keep_question(&p);
        out
    })
}


/// A row that belongs to the row above it: its text lines up with that row's title (an empty 20 px icon), as the
/// rows inside a Windows expander do.
fn sub_row(card: SettingsCard) -> SettingsCard {
    let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    room.set_size_request(20, 20);
    card.set_icon_widget(&room);
    card
}

#[cfg(test)]
mod gnome_tests {
    use super::*;

    fn mode(id: &str, w: i64, h: i64, hz: f64, scales: &[f64], pref: f64, variable: bool) -> GMode {
        GMode { id: id.into(), w, h, hz, scales: scales.to_vec(), preferred_scale: pref, preferred: false, variable }
    }

    fn monitor(c: &str, modes: Vec<GMode>, current: &str) -> GMonitor {
        GMonitor {
            connector: c.into(),
            name: c.into(),
            vendor: String::new(),
            product: String::new(),
            serial: String::new(),
            builtin: false,
            for_lease: false,
            modes,
            current: Some(current.into()),
            preferred: Some(current.into()),
            underscanning: None,
            color_mode: None,
            color_modes: Vec::new(),
            rgb_range: None,
            privacy: false,
        }
    }

    /// The host's fixture (host/src/services/display/fake.rs TWO_MONITORS): eDP-1 at 5/3 left of HDMI-1 at 2.
    fn two() -> GState {
        let s = [1.0, 1.25, 1.5, 1.6666666666666667, 1.75, 2.0];
        let mut edp = monitor(
            "eDP-1",
            vec![
                mode("2560x1600@60.002", 2560, 1600, 60.002, &s, 1.6666666666666667, false),
                mode("2560x1600@120.000", 2560, 1600, 120.0, &s, 1.6666666666666667, false),
                mode("2560x1600@120.000+vrr", 2560, 1600, 120.0, &s, 1.6666666666666667, true),
                mode("1920x1200@60.000", 1920, 1200, 60.0, &[1.0, 1.25, 1.5], 1.25, false),
                mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0, 1.25], 1.0, false),
            ],
            "2560x1600@60.002",
        );
        edp.color_modes = vec![0, 1];
        edp.color_mode = Some(0);
        edp.builtin = true;
        let mut hdmi = monitor(
            "HDMI-1",
            vec![
                mode("3840x2160@60.000", 3840, 2160, 60.0, &[1.0, 1.25, 1.5, 1.75, 2.0], 2.0, false),
                mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0, 1.25], 1.0, false),
            ],
            "3840x2160@60.000",
        );
        hdmi.underscanning = Some(false);
        GState {
            serial: 7,
            allowed: true,
            physical: false,
            orientation_managed: false,
            monitors: vec![edp, hdmi],
            logical: vec![
                GLogical { x: 0, y: 0, scale: 1.6666666666666667, transform: 0, primary: true, monitors: vec!["eDP-1".into()] },
                GLogical { x: 1536, y: 0, scale: 2.0, transform: 0, primary: false, monitors: vec!["HDMI-1".into()] },
            ],
        }
    }

    #[test]
    fn the_state_parses_as_the_host_sends_it() {
        let v = json!({"supported": true, "serial": 7, "allowed": false, "layoutMode": 1, "panelOrientationManaged": true,
            "monitors": [{"connector": "eDP-1", "displayName": "", "builtin": true, "underscanning": null, "colorModes": [0, 1], "colorMode": 0,
                "privacyScreen": {"enabled": false, "locked": false}, "current": "a", "preferred": "a",
                "modes": [{"id": "a", "width": 1920, "height": 1080, "refresh": 60.0, "preferredScale": 1.0, "scales": [1.0, 1.25], "variable": false}]}],
            "logical": [{"x": 0, "y": 0, "scale": 1.0, "transform": 0, "primary": true, "monitors": ["eDP-1"]}]});
        let st = GState::parse(&v);
        assert!(!st.allowed && st.orientation_managed && !st.physical);
        assert_eq!(st.monitors[0].name, "eDP-1", "an empty display name falls back to the connector");
        assert!(st.monitors[0].privacy && st.monitors[0].hdr_capable());
        assert_eq!(st.monitors[0].underscanning, None);
        assert_eq!(st.logical_of("eDP-1"), Some(0));
        assert_eq!(st.number("eDP-1"), 1);
    }

    #[test]
    fn the_current_layout_carries_every_monitor_property() {
        let st = two();
        let cfg = config_of(&st);
        assert_eq!(cfg.len(), 2);
        assert_eq!(cfg[0].monitors[0].color_mode, Some(0), "HDR off is sent, or mutter would reset it");
        assert_eq!(cfg[1].monitors[0].underscanning, Some(false));
        assert_eq!(cfg[0].monitors[0].underscanning, None, "not supported: not sent");
        let a = args_of(&st, &cfg);
        assert_eq!(a["serial"], 7);
        assert_eq!(a["method"], "persistent");
        assert_eq!(a["logical"][1]["x"], 1536);
        assert_eq!(a["logical"][0]["monitors"][0]["props"], json!({"color-mode": 0}));
        assert_eq!(a["logical"][1]["monitors"][0]["props"], json!({"underscanning": false}));
        assert_eq!(rects(&st, &cfg), vec![(0, 0, 1536, 960), (1536, 0, 1920, 1080)]);
    }

    #[test]
    fn a_new_scale_keeps_the_monitors_side_by_side() {
        let st = two();
        let cfg = with_scale(&st, &config_of(&st), "eDP-1", 2.0).unwrap();
        assert_eq!(cfg[0].scale, 2.0);
        assert_eq!((cfg[1].x, cfg[1].y), (1280, 0), "HDMI-1 moves to eDP-1's new right edge");
        // stacked: the lower one follows the upper one's new height
        let old = [(0, 0, 1920, 1080), (0, 1080, 1920, 1080)];
        assert_eq!(relayout(&old, &[(1536, 864), (1920, 1080)]), vec![(0, 0), (0, 864)]);
        // a monitor left of the primary (negative x) ends at 0,0
        let old = [(-1920, 0, 1920, 1080), (0, 0, 1536, 960)];
        assert_eq!(relayout(&old, &[(1280, 720), (1536, 960)]), vec![(0, 0), (1280, 0)]);
        // the orientation turns the size
        let cfg = with_transform(&st, &config_of(&st), "HDMI-1", 1).unwrap();
        assert_eq!(rects(&st, &cfg)[1], (1536, 0, 1080, 1920));
    }

    #[test]
    fn a_new_resolution_keeps_a_scale_the_mode_supports() {
        let st = two();
        let cfg = with_mode(&st, &config_of(&st), "eDP-1", "1920x1200@60.000").unwrap();
        assert_eq!(cfg[0].monitors[0].mode, "1920x1200@60.000");
        assert_eq!(cfg[0].scale, 1.25, "5/3 is not offered for 1920×1200: its preferred scale");
        assert_eq!(cfg[1].x, 1536, "1920/1.25");
        let cfg = with_mode(&st, &config_of(&st), "eDP-1", "2560x1600@120.000").unwrap();
        assert_eq!(cfg[0].scale, 1.6666666666666667, "the same size keeps its scale");
        assert!(with_mode(&st, &config_of(&st), "eDP-1", "nope").is_none());
    }

    #[test]
    fn presentation_modes_build_mirror_extend_and_only() {
        let st = two();
        assert_eq!(presentation(&st), "extend");
        let mirror = mirror_config(&st).unwrap();
        assert_eq!(mirror.len(), 1);
        assert_eq!(mirror[0].monitors.len(), 2);
        assert_eq!(mirror[0].monitors[0].mode, "1920x1080@60.000", "the largest size both have");
        assert_eq!(mirror[0].monitors[1].mode, "1920x1080@60.000");
        assert_eq!(mirror[0].scale, 1.0, "5/3 is not offered at 1920×1080");
        assert!(mirror[0].primary);
        // a mirror state: one logical monitor with both
        let mut m = st.clone();
        m.logical = vec![GLogical { x: 0, y: 0, scale: 1.0, transform: 0, primary: true, monitors: vec!["eDP-1".into(), "HDMI-1".into()] }];
        assert_eq!(presentation(&m), "mirror");
        let ext = extend_config(&m).unwrap();
        assert_eq!(ext.len(), 2);
        assert_eq!((ext[1].x, ext[1].y), (size_of(&m, &ext[0]).0, 0));
        assert!(ext[0].primary && !ext[1].primary);
        let only = only_config(&st, "HDMI-1").unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].scale, 2.0, "its own scale stays");
        let mut o = st.clone();
        o.logical.truncate(1);
        assert_eq!(presentation(&o), "only:eDP-1");
        assert_eq!(with_primary(&config_of(&st), "HDMI-1").unwrap().iter().map(|l| l.primary).collect::<Vec<_>>(), [false, true]);
    }

    #[test]
    fn a_dragged_monitor_snaps_to_an_edge() {
        let rs = [(0, 0, 1536, 960), (1536, 0, 1920, 1080)];
        // dropped below eDP-1, a little to the right: it snaps under it, left edges lined up
        assert_eq!(snap(&rs, 1, 20, 1000), Some((0, 960)));
        // dropped left of eDP-1
        assert_eq!(snap(&rs, 1, -1800, 20), Some((-1920, 0)));
        let st = two();
        let cfg = with_position(&st, &config_of(&st), "HDMI-1", -1800, 20).unwrap();
        assert_eq!((cfg[0].x, cfg[1].x), (1920, 0), "normalised: the layout starts at 0,0");
    }

    #[test]
    fn a_dragged_middle_monitor_never_leaves_another_alone() {
        // three in a row: the middle one dropped below the left one would leave the right one touching none
        let rs = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080), (3840, 0, 1920, 1080)];
        assert!(connected(&rs));
        let alone = [(0, 0, 1920, 1080), (0, 1080, 1920, 1080), (3840, 0, 1920, 1080)];
        assert!(!connected(&alone), "the right monitor touches none");
        let (x, y) = snap(&rs, 1, 0, 1200).expect("a place that keeps the layout connected");
        assert_ne!((x, y), (0, 1080));
        let mut layout = rs.to_vec();
        layout[1] = (x, y, 1920, 1080);
        assert!(connected(&layout), "{layout:?}");
        assert!((0..3).all(|a| (0..3).all(|b| a == b || !overlaps(layout[a], layout[b]))));
        // corners only touching are no contact (mutter's rule)
        assert!(!adjacent((0, 0, 10, 10), (10, 10, 10, 10)));
        assert!(adjacent((0, 0, 10, 10), (10, 9, 10, 10)));
        // an end monitor may still go under its neighbour: the other two stay together
        assert_eq!(snap(&rs, 2, 1920, 1200), Some((1920, 1080)));
    }

    #[test]
    fn scales_rates_and_custom_scaling() {
        let st = two();
        let edp = &st.monitors[0];
        let md = edp.active_mode().unwrap();
        let opts = scale_options(md, 1.6666666666666667);
        assert_eq!(opts.iter().map(|s| pct(*s)).collect::<Vec<_>>(), ["100%", "125%", "150%", "167%", "175%", "200%"]);
        assert_eq!(nearest_scale(&md.scales, 160.0), Some(1.6666666666666667));
        assert_eq!(nearest_scale(&md.scales, 500.0), Some(2.0));
        let rates = rate_modes(edp, md);
        assert_eq!(rates.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["2560x1600@120.000", "2560x1600@60.002"], "fixed rates only");
        assert_eq!(rate_text(60.002), "60 Hz");
        assert!(rate_text(59.951).starts_with("59"));
        let rows = info_rows(&st, edp);
        assert_eq!(rows[0].1, "1536 × 960, 60 Hz");
        assert_eq!(rows[1].1, "2560 × 1600, 60 Hz");
        assert!(rows.iter().all(|(k, _)| !k.is_empty()));
        assert!(with_props(&config_of(&st), "eDP-1", None, Some(1)).unwrap()[0].monitors[0].color_mode == Some(1));
    }

    #[test]
    fn one_keep_question_per_change_and_an_answered_id_asks_again() {
        let mut q = OpenQuestions::default();
        assert!(q.ask(7), "display.apply's answer asks");
        assert!(!q.ask(7), "the event display.confirmChange for the same change does not ask again");
        q.answered(7);
        // review D-1: after a screen lock an older bridge numbered from 1 again; an answered id is no duplicate
        assert!(q.ask(7), "the same id after the answer is a new change");
        assert!(q.ask(8) && !q.ask(8));
        for id in 100..140 {
            q.ask(id);
        }
        assert!(q.ask(8), "only the last 32 open questions are kept");
    }
}
