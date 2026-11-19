//! Instellingen › Systeem › Geluid › Eigenschappen (data page `sound-device`, params {kind, id, card?}) and › Alle
//! geluidsapparaten (`sound-devices`, amendment 1), batch D, notes/claude/settings-parity.md §2, on GNOME's Sound
//! panel (host sound.*):
//!
//! * Eigenschappen, as Windows' device properties: Audio (Toestaan / Niet toestaan = the card's "off" profile and
//!   back), Instellen als standaardgeluidsapparaat, then Uitvoerinstellingen / Invoerinstellingen: Indeling (GNOME's
//!   Configuration, the card profile) with Testen (GNOME's speaker test per channel), Poort, Volume, Linkerkanaal and
//!   Rechterkanaal (GNOME's balance), Balans voor en achter and Subwoofer (GNOME's fade and LFE, surround only),
//!   Volume boven 100% toestaan (GNOME's Overamplification); for an input Invoervolume and Je microfoon testen (the
//!   level meter).
//! * Alle geluidsapparaten: every output and input device, also the cards set to Niet toestaan, so a device turned
//!   off can be turned on again.
use super::sound::{self, Meter, Target, VolumeControl, AMPLIFY};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::Gs;
use kit::controls::{self as c, Chevron, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Slider, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const PAGES: &[(&str, Builder)] = &[("sound-device", sound_device), ("sound-devices", sound_devices)];
pub const CSS: &str = "";

/// What an Eigenschappen page shows: GNOME's device entry, its sink or source (None while its card is off or its
/// port's profile is not active) and its card.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Found {
    pub sink: bool,
    pub entry: Option<Value>,
    pub dev: Option<Value>,
    pub card: Option<Value>,
}

/// The page's device in a sound.state (params {kind, id, card?}).
pub(super) fn find(state: &Value, params: &Value) -> Found {
    let sink = params["kind"] != "source";
    let id = params["id"].as_str().unwrap_or("");
    let entry = sound::list(&state[if sink { "outputs" } else { "inputs" }]).into_iter().find(|d| d["id"] == id);
    let dev = match &entry {
        Some(e) => e["device"].as_str().and_then(|n| sound::device(state, sink, n).cloned()),
        None => sound::device(state, sink, id).cloned(),
    };
    let card_name = entry
        .as_ref()
        .and_then(|e| e["card"].as_str().map(str::to_owned))
        .or_else(|| dev.as_ref().and_then(|d| d["card"].as_str().map(str::to_owned)))
        .or_else(|| params["card"].as_str().map(str::to_owned));
    let card = card_name.and_then(|n| sound::list(&state["cards"]).into_iter().find(|c| c["name"] == n.as_str()));
    Found { sink, entry, dev, card }
}

/// The page's name: the entry's title (Luidsprekers), else the device's or the card's description.
pub(super) fn title_of(f: &Found) -> String {
    f.entry
        .as_ref()
        .map(|e| sound::s(&e["title"]))
        .filter(|s| !s.is_empty())
        .or_else(|| f.dev.as_ref().map(|d| sound::s(&d["description"])))
        .or_else(|| f.card.as_ref().map(|c| sound::s(&c["description"])))
        .unwrap_or_default()
}

/// The card profiles Indeling offers: the available ones, without "off" (that is Audio: Niet toestaan).
pub(super) fn formats(card: &Value) -> Vec<(String, String)> {
    sound::list(&card["profiles"])
        .iter()
        .filter(|p| p["available"] == true && p["name"] != "off")
        .map(|p| (sound::s(&p["name"]), sound::s(&p["description"])))
        .collect()
}

/// What makes the page rebuild (anything but volumes).
fn shape(f: &Found) -> String {
    let d = f.dev.as_ref();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        d.map(|d| sound::s(&d["name"])),
        d.map(|d| d["activePort"].clone()),
        d.map(|d| d["default"].clone()),
        d.map(|d| d["ports"].clone()),
        f.card.as_ref().map(|c| c["activeProfile"].clone()),
        f.card.as_ref().map(|c| c["off"].clone()),
        f.entry.as_ref().map(|e| e["default"].clone()),
    )
}

/// The speaker names of GNOME's test window, in Windows' wording.
pub(super) fn channel_label(position: &str) -> &'static str {
    match position {
        "front-left" => t("Linksvoor", "Front left"),
        "front-right" => t("Rechtsvoor", "Front right"),
        "front-center" => t("Middenvoor", "Front centre"),
        "rear-left" => t("Linksachter", "Rear left"),
        "rear-right" => t("Rechtsachter", "Rear right"),
        "rear-center" => t("Middenachter", "Rear centre"),
        "lfe" => t("Subwoofer", "Subwoofer"),
        "side-left" => t("Linkszijde", "Side left"),
        "side-right" => t("Rechtszijde", "Side right"),
        "front-left-of-center" => t("Links van midden", "Front left of centre"),
        "front-right-of-center" => t("Rechts van midden", "Front right of centre"),
        _ => t("Mono", "Mono"),
    }
}

/// GNOME's speaker test (Testen): one button per channel of the sink, each plays its test sound there.
fn test_speakers(p: &Page, dev: &Value) {
    let name = sound::s(&dev["name"]);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::label(t("Klik op een luidspreker om er een testgeluid op af te spelen.", "Select a speaker to play a test sound on it."), "w11-text-2", true));
    let grid = gtk::FlowBox::new();
    grid.add_css_class("st-sound-test-grid");
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_max_children_per_line(3);
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    for ch in sound::list(&dev["channels"]) {
        let position = sound::s(&ch["position"]);
        let b = c::button(channel_label(&position), Variant::Standard);
        b.set_widget_name(&format!("st-sound-test-{position}"));
        let (pp, sink) = (p.clone(), name.clone());
        b.connect_clicked(move |_| {
            let wait = pp.run_with("sound.test", json!({"sink": sink, "channel": position}), Some(t("Het testgeluid kon niet worden afgespeeld", "Couldn’t play the test sound")), true);
            pp.spawn(async move {
                let _ = wait.await;
            });
        });
        grid.insert(&b, -1);
    }
    body.append(&grid);
    Dialog::new(t("Luidsprekers testen", "Test speakers")).body(&body).close_label(t("Sluiten", "Close")).width(460).show(&p.anchor(), |_| {});
}

/// A slider that writes `args(value)` every 60 ms while dragged and once on release, and follows the server
/// unless the user moved it in the last 0.7 s. Laid out as the Volume row's control (the mute button's room, the
/// slider, its value), so Linkerkanaal and Rechterkanaal line up under Volume as in Windows.
#[derive(Clone)]
struct Bound {
    slider: Slider,
    root: gtk::Box,
    value: gtk::Label,
    busy: Rc<Cell<Option<Instant>>>,
}

/// The value text of a channel slider; the fade's -100..100 has none (its two ends are named in the description).
fn bound_text(min: f64, v: f64) -> String {
    if min < 0.0 { String::new() } else { format!("{}", v.round() as i64) }
}

impl Bound {
    fn new(p: &Page, name: &str, min: f64, max: f64, value: f64, args: Rc<dyn Fn(f64) -> Value>) -> Bound {
        let slider = Slider::new(min, max, 1.0, value.clamp(min, max));
        slider.set_value_visible(false);
        slider.widget().set_size_request(220, -1);
        slider.scale().set_widget_name(name);
        // VolumeControl's row: a 36 px mute button, the slider, a 56 px value, 8 px apart
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("st-volume");
        let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        room.set_size_request(36, -1);
        let label = common::value_label(&bound_text(min, value.clamp(min, max)));
        label.add_css_class("w11-tabular");
        label.set_xalign(1.0);
        label.set_size_request(56, -1);
        root.append(&room);
        root.append(slider.widget());
        root.append(&label);
        let busy = Rc::new(Cell::new(None));
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        {
            let (busy, args, label) = (busy.clone(), args.clone(), label.clone());
            slider.connect_changed(move |v| {
                label.set_label(&bound_text(min, v));
                busy.set(Some(Instant::now()));
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let (pend, args) = (pending.clone(), args.clone());
                let id = glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    pend.borrow_mut().take();
                    kit::ctx().fire("sound.setBalance", args(v.round()));
                });
                *pending.borrow_mut() = Some(id);
            });
        }
        {
            let (busy, pp) = (busy.clone(), p.clone());
            slider.connect_committed(move |v| {
                busy.set(Some(Instant::now()));
                let wait = pp.run_with("sound.setBalance", args(v.round()), None, true);
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
        }
        Bound { slider, root, value: label, busy }
    }

    fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The user moved it in the last 700 ms (a server value does not move it back meanwhile).
    fn busy(&self) -> bool {
        self.busy.get().is_some_and(|at| at.elapsed() < Duration::from_millis(700))
    }

    fn update(&self, v: f64) {
        if self.busy() {
            return;
        }
        self.slider.set_value(v.round());
        let min = self.slider.scale().adjustment().lower();
        self.value.set_label(&bound_text(min, self.slider.value()));
    }
}

/// What Linkerkanaal and Rechterkanaal write together (sound.setBalance {left, right}): the server's last channel
/// volumes, the side the user moves with the slider's value. It follows every volume change except on a side the
/// user is moving, so a write never sends a stale other side (review D-4: the host takes the louder side as the
/// volume, so a stale 100 there turned a 20 % sink into 100 %).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Channels {
    left: f64,
    right: f64,
}

impl Channels {
    fn new(left: f64, right: f64) -> Channels {
        Channels { left, right }
    }

    /// The server's values; `busy`: (left, right) is being moved and keeps its own.
    fn follow(&mut self, left: f64, right: f64, busy: (bool, bool)) {
        if !busy.0 {
            self.left = left;
        }
        if !busy.1 {
            self.right = right;
        }
    }

    /// One side moved to `v`: the pair to write.
    fn with(&mut self, left: bool, v: f64) -> (f64, f64) {
        if left {
            self.left = v;
        } else {
            self.right = v;
        }
        (self.left, self.right)
    }
}

/// The controls that follow volume changes without a rebuild.
#[derive(Default)]
struct Live {
    volume: Option<VolumeControl>,
    left: Option<Bound>,
    right: Option<Bound>,
    channels: Option<Rc<RefCell<Channels>>>,
    fade: Option<Bound>,
    lfe: Option<Bound>,
}

struct DevicePage {
    p: Page,
    gs: Gs,
    params: Value,
    state: RefCell<Value>,
    shape: RefCell<String>,
    body: gtk::Box,
    live: RefCell<Live>,
    meter: Meter,
}

fn button_card(icon: &str, title: &str, description: &str, label: &str, name: &str, on: impl Fn() + 'static) -> (SettingsCard, gtk::Button) {
    let card = SettingsCard::new(icon, title, description);
    let b = c::button(label, Variant::Standard);
    b.set_widget_name(name);
    b.connect_clicked(move |_| on());
    card.set_control(&b);
    (card, b)
}

/// Run a write and read the state again when it fails (the page shows the server's state).
fn write(dp: &Rc<DevicePage>, method: &'static str, args: Value, title: &'static str) {
    let wait = dp.p.run_with(method, args, Some(title), true);
    let dp2 = dp.clone();
    dp.p.spawn(async move {
        if wait.await.is_none() && dp2.p.alive() {
            dp2.shape.replace(String::new());
            if let Some(state) = dp2.p.get("sound.state", json!({})).await {
                *dp2.state.borrow_mut() = state;
                render(&dp2);
            }
        }
    });
}

fn render(dp: &Rc<DevicePage>) {
    let state = dp.state.borrow().clone();
    let f = find(&state, &dp.params);
    let shape = shape(&f);
    if *dp.shape.borrow() == shape {
        update(dp, &f);
        return;
    }
    *dp.shape.borrow_mut() = shape;
    *dp.live.borrow_mut() = Live::default();
    let title = title_of(&f);
    if !title.is_empty() {
        dp.p.set_title(&title);
    }
    let mut out: Vec<gtk::Widget> = Vec::new();
    if f.dev.is_none() && f.card.is_none() {
        dp.meter.set(None);
        let bar = common::notice(t("Dit geluidsapparaat is niet meer aangesloten.", "This sound device is no longer connected."));
        bar.widget().set_widget_name("st-sound-device-gone");
        out.push(bar.widget().clone().upcast());
        common::set_slot(&dp.body, &out);
        return;
    }
    let sink = f.sink;
    let top = SettingsGroup::new("", "");
    // Audio: Toestaan / Niet toestaan (the card's off profile, and back to the profile before)
    if let Some(card) = &f.card
        && sound::list(&card["profiles"]).iter().any(|p| p["name"] == "off")
    {
        let off = card["off"] == true;
        let card_name = sound::s(&card["name"]);
        let restore = card["restore"].clone();
        let can_restore = restore.is_string();
        let dp2 = dp.clone();
        let (row, b) = button_card(
            "",
            t("Audio", "Audio"),
            t("Toestaan dat apps en WinPoint dit apparaat gebruiken voor audio", "Allow apps and WinPoint to use this device for audio"),
            if off { t("Toestaan", "Allow") } else { t("Niet toestaan", "Don’t allow") },
            "st-sound-device-audio",
            move || {
                let profile = if off { restore.clone() } else { json!("off") };
                if profile.is_string() {
                    write(&dp2, "sound.setProfile", json!({"card": card_name, "profile": profile}), t("Het geluidsapparaat kon niet worden gewijzigd", "Couldn’t change the sound device"));
                }
            },
        );
        b.set_sensitive(!off || can_restore);
        top.add(row.widget());
    }
    let off = f.card.as_ref().is_some_and(|c| c["off"] == true);
    if off {
        dp.meter.set(None);
        out.push(top.widget().clone().upcast());
        common::set_slot(&dp.body, &out);
        return;
    }
    // Instellen als standaardgeluidsapparaat
    let is_default = f.entry.as_ref().map_or_else(|| f.dev.as_ref().is_some_and(|d| d["default"] == true), |e| e["default"] == true);
    let args = match (&f.entry, &f.dev) {
        (Some(e), _) => Some(sound::default_args(e)),
        (None, Some(d)) => Some(json!({"kind": if sink { "sink" } else { "source" }, "name": d["name"]})),
        _ => None,
    };
    if let Some(args) = args {
        let dp2 = dp.clone();
        let (row, b) = button_card(
            "",
            t("Instellen als standaardgeluidsapparaat", "Set as default sound device"),
            if is_default { t("Dit is het standaardgeluidsapparaat", "This is the default sound device") } else { "" },
            t("Instellen als standaard", "Set as default"),
            "st-sound-device-default",
            move || write(&dp2, "sound.setDefault", args.clone(), t("Het geluidsapparaat kon niet worden gewijzigd", "Couldn’t change the sound device")),
        );
        b.set_sensitive(!is_default);
        top.add(row.widget());
    }
    out.push(top.widget().clone().upcast());

    let group = SettingsGroup::new(if sink { t("Uitvoerinstellingen", "Output settings") } else { t("Invoerinstellingen", "Input settings") }, "");
    // Indeling (GNOME's Configuration: the card profile) with Testen
    if let Some(card) = &f.card {
        let options = formats(card);
        if !options.is_empty() {
            let opts: Vec<DropdownOption> = options.iter().map(|(n, d)| DropdownOption::new(n, d)).collect();
            let dd = Dropdown::new(opts, card["activeProfile"].as_str());
            dd.widget().set_widget_name("st-sound-device-format");
            let (dp2, card_name) = (dp.clone(), sound::s(&card["name"]));
            dd.connect_changed(move |profile| {
                write(&dp2, "sound.setProfile", json!({"card": card_name, "profile": profile}), t("De indeling kon niet worden gewijzigd", "Couldn’t change the format"));
            });
            let row = SettingsCard::new("", t("Indeling", "Format"), "");
            row.set_control(dd.widget());
            if sink && let Some(dev) = &f.dev {
                let test = c::button(t("Testen", "Test"), Variant::Standard);
                test.set_widget_name("st-sound-device-test");
                let (pp, d) = (dp.p.clone(), dev.clone());
                test.connect_clicked(move |_| test_speakers(&pp, &d));
                row.add_control(&test);
            }
            group.add(row.widget());
        }
    }
    let Some(dev) = f.dev.clone() else {
        dp.meter.set(None);
        out.push(group.widget().clone().upcast());
        common::set_slot(&dp.body, &out);
        return;
    };
    let name = sound::s(&dev["name"]);
    // Poort
    let ports: Vec<Value> = sound::list(&dev["ports"]).into_iter().filter(|p| p["available"] != "no").collect();
    if ports.len() > 1 {
        let opts: Vec<DropdownOption> = ports.iter().map(|p| DropdownOption::new(&sound::s(&p["name"]), &sound::s(&p["description"]))).collect();
        let dd = Dropdown::new(opts, dev["activePort"].as_str());
        dd.widget().set_widget_name("st-sound-device-port");
        let (dp2, n) = (dp.clone(), name.clone());
        dd.connect_changed(move |port| {
            write(&dp2, "sound.setPort", json!({"kind": if sink { "sink" } else { "source" }, "name": n, "port": port}), t("De poort kon niet worden gewijzigd", "Couldn’t change the port"));
        });
        let row = SettingsCard::new("", t("Poort", "Port"), "");
        row.set_control(dd.widget());
        group.add(row.widget());
    }
    // Volume / Invoervolume
    let max = sound::max_volume(&dp.gs, &state);
    let kind = if sink { "sink" } else { "source" };
    let vc = VolumeControl::new(&dp.p, "st-sound-device-volume", !sink, dev["volume"].as_f64().unwrap_or(0.0), dev["mute"] == true, max, Target::Device(kind, name.clone()), true);
    let row = SettingsCard::new("", if sink { t("Volume", "Volume") } else { t("Invoervolume", "Input volume") }, "");
    row.set_control(vc.widget());
    group.add(row.widget());
    let mut live = Live { volume: Some(vc), ..Live::default() };
    if sink {
        // Linkerkanaal / Rechterkanaal (GNOME's balance)
        if dev["canBalance"] == true {
            let (cur_l, cur_r) = (dev["left"].as_f64().unwrap_or(0.0), dev["right"].as_f64().unwrap_or(0.0));
            let lr = Rc::new(RefCell::new(Channels::new(cur_l, cur_r)));
            for (left, title, widget) in [(true, t("Linkerkanaal", "Left channel"), "st-sound-device-left"), (false, t("Rechterkanaal", "Right channel"), "st-sound-device-right")] {
                let (n, lr2) = (name.clone(), lr.clone());
                let b = Bound::new(&dp.p, widget, 0.0, max.round(), if left { cur_l } else { cur_r }, Rc::new(move |v| {
                    let (l, r) = lr2.borrow_mut().with(left, v);
                    json!({"sink": n, "left": l, "right": r})
                }));
                let row = SettingsCard::new("", title, "");
                row.set_control(b.widget());
                group.add(row.widget());
                if left {
                    live.left = Some(b);
                } else {
                    live.right = Some(b);
                }
            }
            live.channels = Some(lr);
        }
        // GNOME's fade and subwoofer (surround)
        if dev["canFade"] == true {
            let n = name.clone();
            let b = Bound::new(&dp.p, "st-sound-device-fade", -100.0, 100.0, dev["fade"].as_f64().unwrap_or(0.0) * 100.0, Rc::new(move |v| json!({"sink": n, "fade": v / 100.0})));
            let row = SettingsCard::new("", t("Balans voor en achter", "Front and rear balance"), t("Naar links klinkt achter luider, naar rechts voor", "To the left the rear is louder, to the right the front"));
            row.set_control(b.widget());
            group.add(row.widget());
            live.fade = Some(b);
        }
        if dev["hasLfe"] == true {
            let n = name.clone();
            let b = Bound::new(&dp.p, "st-sound-device-lfe", 0.0, max.round(), dev["lfe"].as_f64().unwrap_or(100.0), Rc::new(move |v| json!({"sink": n, "lfe": v})));
            let row = SettingsCard::new("", t("Subwoofer", "Subwoofer"), "");
            row.set_control(b.widget());
            group.add(row.widget());
            live.lfe = Some(b);
        }
        if let Some(card) = dp.gs.toggle_card(&AMPLIFY, "", t("Volume boven 100% toestaan", "Allow volume above 100%"), t("Met verminderde geluidskwaliteit", "With reduced sound quality"), false) {
            group.add(card.widget());
        }
        dp.meter.set(None);
    } else {
        // Je microfoon testen: the level meter while the test runs
        let row = SettingsCard::new("", t("Je microfoon testen", "Test your microphone"), t("Spreek in je microfoon: de balk laat zien wat hij hoort", "Speak into your microphone: the bar shows what it hears"));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.set_valign(gtk::Align::Center);
        let b = c::button(t("Test starten", "Start test"), Variant::Standard);
        b.set_widget_name("st-sound-device-mic-test");
        b.set_halign(gtk::Align::End);
        column.append(&b);
        let bar = dp.meter.widget();
        if let Some(parent) = bar.parent()
            && let Some(pb) = parent.downcast_ref::<gtk::Box>()
        {
            pb.remove(bar);
        }
        bar.set_size_request(220, -1);
        column.append(bar);
        row.set_control(&column);
        let testing = Rc::new(Cell::new(false));
        let (meter, n) = (dp.meter.clone(), name.clone());
        b.connect_clicked(move |b| {
            let on = !testing.get();
            testing.set(on);
            meter.set(on.then_some(n.as_str()));
            b.set_label(if on { t("Test stoppen", "Stop test") } else { t("Test starten", "Start test") });
        });
        group.add(row.widget());
        dp.meter.set(None);
    }
    out.push(group.widget().clone().upcast());
    *dp.live.borrow_mut() = live;
    common::set_slot(&dp.body, &out);
}

/// A volume change only: the sliders move, nothing is rebuilt.
fn update(dp: &Rc<DevicePage>, f: &Found) {
    let Some(dev) = &f.dev else { return };
    let live = dp.live.borrow();
    if let Some(vc) = &live.volume {
        vc.update(dev["volume"].as_f64().unwrap_or(0.0), dev["mute"] == true);
    }
    if let Some(b) = &live.left {
        b.update(dev["left"].as_f64().unwrap_or(0.0));
    }
    if let Some(b) = &live.right {
        b.update(dev["right"].as_f64().unwrap_or(0.0));
    }
    if let Some(lr) = &live.channels {
        let busy = |b: &Option<Bound>| b.as_ref().is_some_and(Bound::busy);
        lr.borrow_mut().follow(dev["left"].as_f64().unwrap_or(0.0), dev["right"].as_f64().unwrap_or(0.0), (busy(&live.left), busy(&live.right)));
    }
    if let Some(b) = &live.fade {
        b.update(dev["fade"].as_f64().unwrap_or(0.0) * 100.0);
    }
    if let Some(b) = &live.lfe {
        b.update(dev["lfe"].as_f64().unwrap_or(0.0));
    }
}

/// Without a PulseAudio server: the notice and the way back.
fn unavailable(p: &Page) -> Content {
    let bar = common::notice(t("Geluidsapparaten kunnen hier niet worden geregeld.", "Sound devices can’t be controlled here."));
    let group = SettingsGroup::new("", "");
    group.add(common::link_card(p, "sound").widget());
    vec![bar.widget().clone().upcast(), group.widget().clone().upcast()]
}

fn sound_device(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.call("sound.state", json!({}));
        let gs = Gs::load(&p, &[AMPLIFY]).await;
        let state = state.await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if state["available"] != true {
            return unavailable(&p);
        }
        let dp = Rc::new(DevicePage {
            p: p.clone(),
            gs: gs.clone(),
            params: p.params(),
            state: RefCell::new(state),
            shape: RefCell::new(String::new()),
            body: common::slot(),
            live: RefCell::new(Live::default()),
            meter: Meter::new(&p, "source", "st-sound-device-level"),
        });
        render(&dp);
        {
            let dp2 = dp.clone();
            p.on("sound.changed", move |d| {
                if d["available"] != true {
                    dp2.p.refresh();
                    return;
                }
                *dp2.state.borrow_mut() = d.clone();
                render(&dp2);
            });
        }
        {
            let pp = p.clone();
            gs.follow(&AMPLIFY, move |_| pp.refresh());
        }
        vec![dp.body.clone().upcast()]
    })
}

// ------------------------------------------------------------------------------------------------ Alle geluidsapparaten

/// One section of Alle geluidsapparaten: GNOME's entries of that direction, then the cards set to Niet toestaan that
/// have such a side (each opens Eigenschappen).
pub(super) fn all_devices(state: &Value, sink: bool) -> Vec<(String, String, Value, bool)> {
    let mut out: Vec<(String, String, Value, bool)> = sound::list(&state[if sink { "outputs" } else { "inputs" }])
        .iter()
        .map(|d| (sound::s(&d["title"]), sound::s(&d["subtitle"]), sound::props_params(d), false))
        .collect();
    for card in sound::list(&state["cards"]) {
        if card["off"] == true && card[if sink { "outputs" } else { "inputs" }] == true {
            out.push((sound::s(&card["description"]), String::new(), json!({"kind": if sink { "sink" } else { "source" }, "card": card["name"]}), true));
        }
    }
    out
}

fn devices_shape(state: &Value) -> String {
    format!("{}\n{}", serde_json::to_string(&all_devices(state, true)).unwrap_or_default(), serde_json::to_string(&all_devices(state, false)).unwrap_or_default())
}

fn sound_devices(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.get("sound.state", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if state["available"] != true {
            return unavailable(&p);
        }
        let mut out: Content = Vec::new();
        for (sink, title, prefix) in [(true, t("Uitvoerapparaten", "Output devices"), "st-sound-devices-out"), (false, t("Invoerapparaten", "Input devices"), "st-sound-devices-in")] {
            let group = SettingsGroup::new(title, "");
            let entries = all_devices(&state, sink);
            if entries.is_empty() {
                group.add(SettingsCard::new("", if sink { t("Geen geluidsapparaten gevonden", "No sound devices found") } else { t("Geen invoerapparaten gevonden", "No input devices found") }, "").widget());
            }
            for (n, (title, subtitle, params, off)) in entries.into_iter().enumerate() {
                let glyph = if sink { "sound" } else { "mic" };
                let desc = if off { t("Uitgeschakeld", "Disabled").to_owned() } else { subtitle };
                let card = SettingsCard::clickable(glyph, &title, &desc, Chevron::Right);
                if let Some(b) = card.button() {
                    b.set_widget_name(&format!("{prefix}-{n}"));
                }
                let pp = p.clone();
                card.connect_clicked(move || pp.go_with("sound-device", params.clone()));
                group.add(card.widget());
            }
            out.push(group.widget().clone().upcast());
        }
        let shape = Rc::new(RefCell::new(devices_shape(&state)));
        let pp = p.clone();
        p.on("sound.changed", move |d| {
            if devices_shape(d) != *shape.borrow() {
                *shape.borrow_mut() = devices_shape(d);
                pp.refresh();
            }
        });
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_sliders_show_their_value_but_the_fade_does_not() {
        assert_eq!(bound_text(0.0, 49.6), "50");
        assert_eq!(bound_text(0.0, 153.0), "153");
        assert_eq!(bound_text(-100.0, 20.0), "", "the fade's ends are named in its description");
    }

    fn state() -> Value {
        json!({
            "available": true,
            "outputs": [
                {"id": "card0:speaker", "kind": "sink", "title": "Speakers", "subtitle": "Built-in Audio", "device": "sink0", "port": "speaker", "card": "card0", "profile": null, "default": true},
                {"id": "card0:hdmi", "kind": "sink", "title": "HDMI / DisplayPort", "subtitle": "Built-in Audio", "device": null, "port": "hdmi", "card": "card0", "profile": "output:hdmi-stereo", "default": false}
            ],
            "inputs": [],
            "sinks": [{"name": "sink0", "description": "Built-in Audio Analog Stereo", "card": "card0", "default": true}],
            "sources": [],
            "cards": [
                {"name": "card0", "description": "Built-in Audio", "off": false, "outputs": true, "inputs": true, "activeProfile": "output:analog-stereo", "restore": "output:analog-stereo",
                 "profiles": [{"name": "off", "description": "Off", "available": true}, {"name": "output:analog-stereo", "description": "Analog Stereo Output", "available": true}, {"name": "output:hdmi-stereo", "description": "Digital Stereo (HDMI) Output", "available": false}]},
                {"name": "usb0", "description": "USB Headset", "off": true, "outputs": true, "inputs": true, "activeProfile": "off", "restore": "output:analog-stereo+input:mono", "profiles": []}
            ]
        })
    }

    #[test]
    fn the_properties_page_finds_its_entry_device_and_card() {
        let s = state();
        let f = find(&s, &json!({"kind": "sink", "id": "card0:speaker", "card": "card0"}));
        assert_eq!(f.dev.as_ref().map(|d| d["name"].clone()), Some(json!("sink0")));
        assert_eq!(f.card.as_ref().map(|c| c["name"].clone()), Some(json!("card0")));
        assert_eq!(title_of(&f), "Speakers");
        let hdmi = find(&s, &json!({"kind": "sink", "id": "card0:hdmi", "card": "card0"}));
        assert!(hdmi.entry.is_some() && hdmi.dev.is_none(), "a port of another profile has no sink yet");
        // after Niet toestaan the entry is gone, the card stays
        let off = find(&s, &json!({"kind": "sink", "id": "usb0:analog-output", "card": "usb0"}));
        assert!(off.entry.is_none() && off.dev.is_none());
        assert_eq!(off.card.as_ref().map(|c| c["off"].clone()), Some(json!(true)));
        assert_eq!(title_of(&off), "USB Headset");
        assert_eq!(find(&s, &json!({"kind": "sink", "id": "gone"})), Found { sink: true, ..Found::default() });
        assert_eq!(find(&s, &json!({"kind": "sink", "id": "sink0"})).dev.map(|d| d["name"].clone()), Some(json!("sink0")), "a device without an entry by name");
    }

    #[test]
    fn indeling_offers_the_available_profiles_without_off() {
        let s = state();
        assert_eq!(formats(&s["cards"][0]), vec![("output:analog-stereo".to_owned(), "Analog Stereo Output".to_owned())]);
    }

    #[test]
    fn all_sound_devices_lists_the_turned_off_cards_too() {
        let s = state();
        let out = all_devices(&s, true);
        let titles: Vec<&str> = out.iter().map(|(t, _, _, _)| t.as_str()).collect();
        assert_eq!(titles, ["Speakers", "HDMI / DisplayPort", "USB Headset"]);
        assert_eq!(out[2].2, json!({"kind": "sink", "card": "usb0"}));
        assert!(out[2].3);
        assert_eq!(all_devices(&s, false).len(), 1, "the headset's input side");
    }

    #[test]
    fn a_channel_write_keeps_the_other_channel_at_the_servers_value() {
        // review D-4: the page opens at 100/100, the volume goes to 20 elsewhere, then Rechterkanaal moves to 15
        let mut lr = Channels::new(100.0, 100.0);
        lr.follow(20.0, 20.0, (false, false));
        assert_eq!(lr.with(false, 15.0), (20.0, 15.0), "the left channel keeps the new volume, not the 100 of the start");
        // while the user drags the right side the server's echo does not take it back, the left side still follows
        lr.follow(30.0, 20.0, (false, true));
        assert_eq!(lr, Channels::new(30.0, 15.0));
        assert_eq!(lr.with(true, 40.0), (40.0, 15.0));
    }

    #[test]
    fn every_channel_has_a_speaker_name() {
        for p in ["front-left", "front-right", "front-center", "rear-left", "rear-right", "rear-center", "lfe", "side-left", "side-right", "front-left-of-center", "front-right-of-center", "mono"] {
            assert!(!channel_label(p).is_empty());
        }
        assert_ne!(channel_label("front-left"), channel_label("front-right"));
    }
}
