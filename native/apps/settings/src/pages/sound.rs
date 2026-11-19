//! Instellingen › Systeem › Geluid (batch D, notes/claude/settings-parity.md §2, amendments 1-3, 39) in Windows 11's
//! layout on GNOME's Sound panel (host sound.*, libpulse on pipewire-pulse): Uitvoer (Kies waar je geluid wilt
//! afspelen with GNOME's device list and a chevron to Eigenschappen, Een nieuw uitvoerapparaat koppelen, Volume),
//! Invoer (Kies een apparaat om te spreken of op te nemen, Een nieuw invoerapparaat koppelen, Volume with the level
//! meter), Geavanceerd (Alle geluidsapparaten, Volumemixer, Meer geluidsinstellingen: the Geluiden tab with GNOME's
//! alert sound and Ubuntu's startup sound). Without a PulseAudio server the page keeps the old system.audio one
//! (6ebf169f:vendor/ui/js/apps/settings/pages/sound.js). The helpers the other sound pages share are `pub(super)`.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Chevron, Dialog, Dropdown, DropdownOption, Pivot, ProgressBar, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, Slider, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const PAGES: &[(&str, Builder)] = &[("sound", sound)];
pub const CSS: &str = ".w11-settings-expander .w11-expander-inner > .st-sound-device.w11-nested-other { padding: 6px 12px 6px 20px; }
checkbutton.st-sound-pick > radio { margin-right: 12px; }
.st-sound-device-sub { color: var(--text-2); font-size: var(--fs-caption); }
.st-level { margin-top: 6px; }
.st-sound-test-grid button { min-width: 132px; }";

/// GNOME's Overamplification (one owner: allow/system.rs).
pub(super) const AMPLIFY: GKey = gs::key("org.gnome.desktop.sound", "allow-volume-above-100-percent");

/// The slider's maximum: 100 %, or GNOME's amplified maximum (sound.state maxVolume) while Overamplification is on.
pub(super) fn max_volume(gs: &Gs, state: &Value) -> f64 {
    if gs.value(&AMPLIFY) == true { state["maxVolume"].as_f64().unwrap_or(100.0).max(100.0) } else { 100.0 }
}

pub(super) fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

pub(super) fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

/// A sink or source of the state by name.
pub(super) fn device<'a>(state: &'a Value, sink: bool, name: &str) -> Option<&'a Value> {
    state[if sink { "sinks" } else { "sources" }].as_array()?.iter().find(|d| d["name"] == name)
}

/// The glyph of a device (Windows shows speakers, headphones, a display, a microphone).
pub(super) fn device_glyph(dev: &Value, sink: bool) -> &'static str {
    if !sink {
        return "mic";
    }
    let form = s(&dev["formFactor"]);
    let port = s(&dev["port"]);
    if form == "headphone" || form == "headset" || port.contains("headphone") || port.contains("headset") {
        "headphones"
    } else if port.contains("hdmi") || port.contains("iec958") || form == "tv" {
        "display"
    } else {
        "sound"
    }
}

// ------------------------------------------------------------------------------------------------ volume control

/// What a volume control writes.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Target {
    /// a sink or source by name
    Device(&'static str, String),
    /// an app's stream ("sinkInput" / "sourceOutput") by index
    Stream(&'static str, u64),
    /// Systeemgeluiden (the event role)
    Event,
}

impl Target {
    fn volume(&self, v: f64) -> (&'static str, Value) {
        match self {
            Target::Device(kind, name) => ("sound.setVolume", json!({"kind": kind, "name": name, "volume": v})),
            Target::Stream(kind, index) => ("sound.setVolume", json!({"kind": kind, "index": index, "volume": v})),
            Target::Event => ("sound.setEventVolume", json!({"volume": v})),
        }
    }

    fn mute(&self, mute: bool, v: f64) -> (&'static str, Value) {
        match self {
            Target::Device(kind, name) => ("sound.setMute", json!({"kind": kind, "name": name, "mute": mute})),
            Target::Stream(kind, index) => ("sound.setMute", json!({"kind": kind, "index": index, "mute": mute})),
            Target::Event => ("sound.setEventVolume", json!({"volume": v, "mute": mute})),
        }
    }
}

struct VolInner {
    p: Page,
    root: gtk::Box,
    mute: gtk::Button,
    glyph: gtk::Image,
    value: gtk::Label,
    slider: Slider,
    muted: Cell<bool>,
    busy: Cell<Option<Instant>>,
    mic: bool,
    target: RefCell<Target>,
    pending: RefCell<Option<glib::SourceId>>,
}

/// Windows' volume row control: the mute button, the slider and its value. A drag writes every 60 ms and once more
/// on release; the server's changes move it again unless the user touched it in the last 0.7 s.
#[derive(Clone)]
pub(super) struct VolumeControl(Rc<VolInner>);

impl VolumeControl {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(p: &Page, name: &str, mic: bool, volume: f64, muted: bool, max: f64, target: Target, writable: bool) -> VolumeControl {
        let mute = gtk::Button::new();
        mute.add_css_class("w11-icon-button");
        mute.set_size_request(36, 36);
        mute.set_valign(gtk::Align::Center);
        mute.set_widget_name(&format!("{name}-mute"));
        let glyph = kit::icons::glyph(if mic { "mic" } else { "volume" }, 18);
        mute.set_child(Some(&glyph));
        let value = common::value_label("");
        value.add_css_class("w11-tabular");
        value.set_xalign(1.0);
        value.set_size_request(56, -1);
        let top = max.max(100.0).round();
        let slider = Slider::new(0.0, top, 1.0, volume.round().min(top));
        slider.set_value_visible(false);
        slider.widget().set_size_request(220, -1);
        slider.scale().set_widget_name(name);
        slider.set_sensitive(writable);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("st-volume");
        root.append(&mute);
        root.append(slider.widget());
        root.append(&value);
        let vc = VolumeControl(Rc::new(VolInner {
            p: p.clone(),
            root,
            mute: mute.clone(),
            glyph,
            value,
            slider: slider.clone(),
            muted: Cell::new(muted),
            busy: Cell::new(None),
            mic,
            target: RefCell::new(target),
            pending: RefCell::new(None),
        }));
        vc.paint();
        {
            let weak = Rc::downgrade(&vc.0);
            slider.connect_changed(move |v| {
                let Some(inner) = weak.upgrade() else { return };
                let vc = VolumeControl(inner);
                vc.touch();
                if v > 0.0 && vc.0.muted.get() {
                    vc.0.muted.set(false);
                }
                vc.paint();
                if let Some(id) = vc.0.pending.borrow_mut().take() {
                    id.remove();
                }
                let weak = Rc::downgrade(&vc.0);
                let id = glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.pending.borrow_mut().take();
                        let (method, args) = inner.target.borrow().volume(v.round());
                        kit::ctx().fire(method, args);
                    }
                });
                *vc.0.pending.borrow_mut() = Some(id);
            });
        }
        {
            let weak = Rc::downgrade(&vc.0);
            slider.connect_committed(move |v| {
                let Some(inner) = weak.upgrade() else { return };
                let vc = VolumeControl(inner);
                vc.touch();
                vc.paint();
                let (method, args) = vc.0.target.borrow().volume(v.round());
                let wait = vc.0.p.run_with(method, args, None, true);
                vc.0.p.spawn(async move {
                    let _ = wait.await;
                });
            });
        }
        {
            let weak = Rc::downgrade(&vc.0);
            mute.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let vc = VolumeControl(inner);
                vc.touch();
                let to = !vc.0.muted.get();
                vc.0.muted.set(to);
                vc.paint();
                let (method, args) = vc.0.target.borrow().mute(to, vc.0.slider.value().round());
                let wait = vc.0.p.run_with(method, args, None, true);
                let vc2 = vc.clone();
                vc.0.p.spawn(async move {
                    if wait.await.is_none() {
                        vc2.0.muted.set(!to);
                        vc2.paint();
                    }
                });
            });
        }
        mute.set_sensitive(writable);
        vc
    }

    fn touch(&self) {
        self.0.busy.set(Some(Instant::now()));
    }

    fn busy(&self) -> bool {
        self.0.busy.get().is_some_and(|at| at.elapsed() < Duration::from_millis(700))
    }

    fn paint(&self) {
        let (v, muted) = (self.0.slider.value(), self.0.muted.get());
        let glyph = if self.0.mic { "mic" } else { kit::icons::volume_icon(v, muted) };
        kit::icons::set_glyph(&self.0.glyph, glyph);
        self.0.mute.set_tooltip_text(Some(if muted { t("Dempen opheffen", "Unmute") } else { t("Dempen", "Mute") }));
        if muted {
            self.0.mute.add_css_class("st-muted");
        } else {
            self.0.mute.remove_css_class("st-muted");
        }
        self.0.value.set_label(&if muted { t("Gedempt", "Muted").to_owned() } else { format!("{}", v.round() as i64) });
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.0.root
    }

    /// The server's volume and mute (ignored while the user is moving it).
    pub(super) fn update(&self, volume: f64, muted: bool) {
        if self.busy() {
            return;
        }
        self.0.slider.set_value(volume.round());
        self.0.muted.set(muted);
        self.paint();
    }
}

// ------------------------------------------------------------------------------------------------ level meter

struct MeterInner {
    p: Page,
    kind: &'static str,
    name: RefCell<Option<String>>,
    bar: ProgressBar,
}

/// A level meter (sound.meter) on one device while the page is shown: renewed every 10 s, stopped when the page
/// goes or the device changes; the bar follows sound.level.
#[derive(Clone)]
pub(super) struct Meter(Rc<MeterInner>);

impl Meter {
    pub(super) fn new(p: &Page, kind: &'static str, widget_name: &str) -> Meter {
        let bar = ProgressBar::new();
        bar.widget().add_css_class("st-level");
        bar.widget().set_widget_name(widget_name);
        let m = Meter(Rc::new(MeterInner { p: p.clone(), kind, name: RefCell::new(None), bar }));
        let weak = Rc::downgrade(&m.0);
        p.on("sound.level", move |d| {
            let Some(inner) = weak.upgrade() else { return };
            if d["kind"] == inner.kind && inner.name.borrow().as_deref() == d["name"].as_str() {
                inner.bar.set_fraction(d["peak"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0));
            }
        });
        let weak = Rc::downgrade(&m.0);
        p.interval(10_000, move || {
            let Some(inner) = weak.upgrade() else { return false };
            if let Some(name) = inner.name.borrow().as_ref() {
                kit::ctx().fire("sound.meter", json!({"kind": inner.kind, "name": name, "on": true}));
            }
            true
        });
        let weak = Rc::downgrade(&m.0);
        p.on_cleanup(move || {
            if let Some(inner) = weak.upgrade()
                && let Some(name) = inner.name.borrow_mut().take()
            {
                kit::ctx().fire("sound.meter", json!({"kind": inner.kind, "name": name, "on": false}));
            }
        });
        m
    }

    pub(super) fn widget(&self) -> &gtk::DrawingArea {
        self.0.bar.widget()
    }

    /// Meter this device (None: stop).
    pub(super) fn set(&self, name: Option<&str>) {
        if self.0.name.borrow().as_deref() == name {
            return;
        }
        if let Some(old) = self.0.name.borrow_mut().take() {
            kit::ctx().fire("sound.meter", json!({"kind": self.0.kind, "name": old, "on": false}));
        }
        self.0.bar.set_fraction(0.0);
        if let Some(n) = name
            && self.0.p.alive()
        {
            *self.0.name.borrow_mut() = Some(n.to_owned());
            kit::ctx().fire("sound.meter", json!({"kind": self.0.kind, "name": n, "on": true}));
        }
    }
}

// ------------------------------------------------------------------------------------------------ device lists

/// sound.setDefault's arguments for one of GNOME's device entries (a port of another profile switches it first).
pub(super) fn default_args(dev: &Value) -> Value {
    let mut a = json!({"kind": dev["kind"]});
    for (from, to) in [("device", "name"), ("port", "port"), ("card", "card"), ("profile", "profile")] {
        if dev[from].is_string() {
            a[to] = dev[from].clone();
        }
    }
    a
}

/// The entries' identity (a list that changes shape is built again, else only the selection moves).
pub(super) fn signature(devices: &[Value]) -> String {
    devices.iter().map(|d| format!("{}|{}|{}", s(&d["id"]), s(&d["title"]), s(&d["subtitle"]))).collect::<Vec<_>>().join("\n")
}

/// The params of the Eigenschappen page for an entry (its card too: after Niet toestaan the entry is gone and the
/// page shows the card with Toestaan).
pub(super) fn props_params(dev: &Value) -> Value {
    let mut params = json!({"kind": dev["kind"], "id": dev["id"]});
    if dev["card"].is_string() {
        params["card"] = dev["card"].clone();
    }
    params
}

struct ListInner {
    p: Page,
    sink: bool,
    exp: SettingsExpander,
    /// the radio group's hidden leader: every row is a radio, also a single one, and "no default" selects it
    leader: gtk::CheckButton,
    signature: RefCell<String>,
    radios: RefCell<Vec<(String, gtk::CheckButton)>>,
    silent: Cell<bool>,
    devices: RefCell<Vec<Value>>,
}

/// Kies waar je geluid wilt afspelen / Kies een apparaat om te spreken of op te nemen: GNOME's devices as radio rows
/// with a chevron to Eigenschappen, and Een nieuw ... apparaat koppelen (amendment 2) at the end.
#[derive(Clone)]
struct DeviceList(Rc<ListInner>);

impl DeviceList {
    fn new(p: &Page, sink: bool) -> DeviceList {
        let (icon, title) = if sink {
            ("sound", t("Kies waar je geluid wilt afspelen", "Choose where to play sound"))
        } else {
            ("mic", t("Kies een apparaat om te spreken of op te nemen", "Choose a device for speaking or recording"))
        };
        let exp = SettingsExpander::new(icon, title, "");
        exp.header_button().set_widget_name(if sink { "st-sound-outputs" } else { "st-sound-inputs" });
        exp.set_expanded(true);
        DeviceList(Rc::new(ListInner {
            p: p.clone(),
            sink,
            exp,
            leader: gtk::CheckButton::new(),
            signature: RefCell::new(String::from("\u{0}")),
            radios: RefCell::new(Vec::new()),
            silent: Cell::new(false),
            devices: RefCell::new(Vec::new()),
        }))
    }

    fn render(&self, devices: &[Value]) {
        *self.0.devices.borrow_mut() = devices.to_vec();
        let current = devices.iter().find(|d| d["default"] == true);
        self.0.exp.header().set_description(&match current {
            Some(d) => s(&d["title"]),
            None if devices.is_empty() => String::new(),
            None => t("Apps gebruiken het apparaat dat je hier kiest", "Apps use the device you choose here").to_owned(),
        });
        let sig = signature(devices);
        if *self.0.signature.borrow() == sig {
            self.0.silent.set(true);
            match current.and_then(|d| self.0.radios.borrow().iter().find(|(id, _)| d["id"] == id.as_str()).map(|(_, r)| r.clone())) {
                Some(radio) => radio.set_active(true),
                None => self.0.leader.set_active(true),
            }
            self.0.silent.set(false);
            return;
        }
        *self.0.signature.borrow_mut() = sig;
        self.0.exp.clear();
        self.0.radios.borrow_mut().clear();
        let prefix = if self.0.sink { "st-sound-out" } else { "st-sound-in" };
        if devices.is_empty() {
            let none = if self.0.sink { t("Geen geluidsapparaten gevonden", "No sound devices found") } else { t("Geen invoerapparaten gevonden", "No input devices found") };
            self.0.exp.add(&c::label(none, "w11-text-2", true));
        }
        self.0.silent.set(true);
        self.0.leader.set_active(true);
        for (n, dev) in devices.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("st-sound-device");
            let radio = gtk::CheckButton::new();
            radio.add_css_class("w11-radio");
            radio.add_css_class("st-sound-pick");
            radio.set_hexpand(true);
            radio.set_widget_name(&format!("{prefix}-{n}"));
            radio.set_group(Some(&self.0.leader));
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            body.append(&kit::icons::glyph(device_glyph(dev, self.0.sink), 20));
            let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
            texts.set_valign(gtk::Align::Center);
            texts.append(&c::label(&s(&dev["title"]), "w11-sc-title", true));
            let sub = s(&dev["subtitle"]);
            if !sub.is_empty() {
                texts.append(&c::label(&sub, "st-sound-device-sub", true));
            }
            body.append(&texts);
            radio.set_child(Some(&body));
            radio.set_active(dev["default"] == true);
            let props = c::icon_button("chevron-right", t("Eigenschappen", "Properties"), 32);
            props.set_widget_name(&format!("{prefix}-{n}-props"));
            props.set_valign(gtk::Align::Center);
            {
                let (pp, params) = (self.0.p.clone(), props_params(dev));
                props.connect_clicked(move |_| pp.go_with("sound-device", params.clone()));
            }
            {
                let weak = Rc::downgrade(&self.0);
                let id = s(&dev["id"]);
                radio.connect_toggled(move |r| {
                    let Some(inner) = weak.upgrade() else { return };
                    if !r.is_active() || inner.silent.get() {
                        return;
                    }
                    DeviceList(inner).choose(&id);
                });
            }
            row.append(&radio);
            row.append(&props);
            self.0.exp.add(&row);
            self.0.radios.borrow_mut().push((s(&dev["id"]), radio));
        }
        self.0.silent.set(false);
        let (title, name) = if self.0.sink {
            (t("Een nieuw uitvoerapparaat koppelen", "Pair a new output device"), "st-sound-pair-output")
        } else {
            (t("Een nieuw invoerapparaat koppelen", "Pair a new input device"), "st-sound-pair-input")
        };
        let pair = SettingsCard::new("", title, "");
        let add = c::button(t("Apparaat toevoegen", "Add device"), Variant::Standard);
        add.set_widget_name(name);
        let pp = self.0.p.clone();
        add.connect_clicked(move |_| pp.go_with("bluetooth", json!({"add": 1})));
        pair.set_control(&add);
        self.0.exp.add(pair.widget());
    }

    /// The user picked an entry: it becomes the default (a profile switch first when its port needs one).
    fn choose(&self, id: &str) {
        let Some(dev) = self.0.devices.borrow().iter().find(|d| d["id"] == id).cloned() else { return };
        self.0.exp.header().set_description(&s(&dev["title"]));
        let wait = self.0.p.run_with("sound.setDefault", default_args(&dev), Some(t("Het geluidsapparaat kon niet worden gewijzigd", "Couldn’t change the sound device")), true);
        let me = self.clone();
        self.0.p.spawn(async move {
            if wait.await.is_none() && me.0.p.alive() {
                let devices = me.0.devices.borrow().clone();
                *me.0.signature.borrow_mut() = String::from("\u{0}");
                me.render(&devices);
            }
        });
    }
}

// ------------------------------------------------------------------------------------------------ Meer geluidsinstellingen

/// GNOME's alert sounds with Windows' names for them.
fn alert_label(a: &str) -> &'static str {
    match a {
        "none" => t("Geen geluiden", "No sounds"),
        "click" => t("Klik", "Click"),
        "string" => t("Snaar", "String"),
        "swing" => t("Schommel", "Swing"),
        "hum" => t("Brom", "Hum"),
        _ => t("Standaard", "Default"),
    }
}

/// Windows' "Meer geluidsinstellingen" (the classic Geluid dialog), tab Geluiden: Geluidsschema (GNOME's alert sound:
/// Geen geluiden, Standaard, Klik, Snaar, Schommel, Brom) with Testen, and Opstartgeluid afspelen (Ubuntu's startup
/// sound, amendment 3). A choice applies at once and plays, as GNOME's chooser does.
pub(super) fn more_sound_settings(p: &Page) {
    let pp = p.clone();
    p.spawn(async move {
        let Some(a) = pp.run_with("sound.alerts.get", json!({}), None, true).await else { return };
        if !pp.alive() {
            return;
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let tabs = Pivot::new(&[("sounds", t("Geluiden", "Sounds"), "")], Some("sounds"));
        body.append(tabs.widget());
        let intro = c::label(
            t(
                "Een geluidsschema is het geluid dat WinPoint en je apps bij een waarschuwing afspelen.",
                "A sound scheme is the sound that WinPoint and your apps play for an alert.",
            ),
            "w11-text-2",
            true,
        );
        intro.set_max_width_chars(52);
        body.append(&intro);
        let available: Vec<String> = list(&a["available"]).iter().map(s).collect();
        let options: Vec<DropdownOption> = list(&a["choices"])
            .iter()
            .map(s)
            .filter(|c| c == "none" || c == "default" || available.contains(c))
            .map(|c| DropdownOption::new(&c, alert_label(&c)))
            .collect();
        let current = s(&a["alert"]);
        let scheme = Dropdown::new(options, Some(&current));
        scheme.widget().set_widget_name("st-sound-alert");
        scheme.widget().set_sensitive(a["writable"] == true);
        let test = c::button(t("Testen", "Test"), Variant::Standard);
        test.set_widget_name("st-sound-alert-test");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(scheme.widget());
        row.append(&test);
        body.append(&c::field(t("Geluidsschema", "Sound scheme"), &row, None));
        {
            let (p2, sc) = (pp.clone(), scheme.clone());
            scheme.connect_changed(move |value| {
                let value = value.to_owned();
                let wait = p2.run_with("sound.alerts.set", json!({"alert": value}), Some(t("Het waarschuwingsgeluid kon niet worden ingesteld", "Couldn’t set the alert sound")), false);
                let (p3, sc2) = (p2.clone(), sc.clone());
                p2.spawn(async move {
                    match wait.await {
                        Some(v) if v["dryRun"] != true => {
                            drop(p3.call("sound.test", json!({"alert": value})));
                        }
                        Some(_) => {}
                        None => {
                            if let Some(back) = p3.get("sound.alerts.get", json!({})).await {
                                sc2.set_selected(back["alert"].as_str());
                            }
                        }
                    }
                });
            });
        }
        {
            let (p2, sc) = (pp.clone(), scheme.clone());
            test.connect_clicked(move |_| {
                let alert = sc.selected().unwrap_or_else(|| "default".into());
                let wait = p2.run_with("sound.test", json!({"alert": alert}), Some(t("Het testgeluid kon niet worden afgespeeld", "Couldn’t play the test sound")), true);
                p2.spawn(async move {
                    let _ = wait.await;
                });
            });
        }
        if a["startupAvailable"] == true {
            let startup = c::checkbox(t("Opstartgeluid afspelen", "Play startup sound"));
            startup.set_widget_name("st-sound-startup-sound");
            startup.set_active(a["startupSound"] == true);
            startup.set_sensitive(a["startupWritable"] == true);
            if a["startupWritable"] != true {
                startup.set_tooltip_text(Some(gs::locked_text()));
            }
            let silent = Rc::new(Cell::new(false));
            let (p2, s2) = (pp.clone(), silent.clone());
            startup.connect_toggled(move |b| {
                if s2.get() {
                    return;
                }
                let on = b.is_active();
                let wait = p2.run_with("sound.alerts.set", json!({"startupSound": on}), None, false);
                let (b2, s3) = (b.clone(), s2.clone());
                p2.spawn(async move {
                    if wait.await.is_none() {
                        s3.set(true);
                        b2.set_active(!on);
                        s3.set(false);
                    }
                });
            });
            body.append(&startup);
        }
        Dialog::new(t("Geluid", "Sound")).body(&body).close_label(t("Sluiten", "Close")).width(480).show(&pp.anchor(), |_| {});
    });
}

// ------------------------------------------------------------------------------------------------ the page

struct SoundPage {
    p: Page,
    gs: Gs,
    state: RefCell<Value>,
    outputs: DeviceList,
    inputs: DeviceList,
    out_slot: gtk::Box,
    in_slot: gtk::Box,
    out_vol: RefCell<Option<(String, VolumeControl)>>,
    in_vol: RefCell<Option<(String, VolumeControl)>>,
    meter: Meter,
}

/// The Volume card of the default sink or source (rebuilt when the default changes, else only updated).
fn render_volume(sp: &Rc<SoundPage>, sink: bool) {
    let state = sp.state.borrow().clone();
    let name = s(&state[if sink { "defaultSink" } else { "defaultSource" }]);
    let dev = device(&state, sink, &name).cloned();
    let (slot, cell) = if sink { (&sp.out_slot, &sp.out_vol) } else { (&sp.in_slot, &sp.in_vol) };
    if !sink {
        sp.meter.set(dev.as_ref().map(|_| name.as_str()));
    }
    let Some(dev) = dev else {
        *cell.borrow_mut() = None;
        let card = common::unavailable_card(if sink { "volume" } else { "mic" }, t("Volume", "Volume"), None);
        common::set_slot(slot, &[card.widget().clone()]);
        return;
    };
    let (v, muted) = (dev["volume"].as_f64().unwrap_or(0.0), dev["mute"] == true);
    if let Some((current, vc)) = cell.borrow().as_ref()
        && *current == name
    {
        vc.update(v, muted);
        return;
    }
    let kind = if sink { "sink" } else { "source" };
    let widget = if sink { "st-sound-volume" } else { "st-sound-input-volume" };
    let vc = VolumeControl::new(&sp.p, widget, !sink, v, muted, max_volume(&sp.gs, &state), Target::Device(kind, name.clone()), true);
    let card = SettingsCard::new(if sink { "volume" } else { "mic" }, t("Volume", "Volume"), "");
    if sink {
        card.set_control(vc.widget());
    } else {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_valign(gtk::Align::Center);
        column.append(vc.widget());
        let bar = sp.meter.widget();
        if let Some(parent) = bar.parent()
            && let Some(b) = parent.downcast_ref::<gtk::Box>()
        {
            b.remove(bar);
        }
        bar.set_margin_start(44);
        bar.set_margin_end(64);
        column.append(bar);
        card.set_control(&column);
    }
    common::set_slot(slot, &[card.widget().clone()]);
    *cell.borrow_mut() = Some((name, vc));
}

fn render(sp: &Rc<SoundPage>) {
    let state = sp.state.borrow().clone();
    sp.outputs.render(&list(&state["outputs"]));
    sp.inputs.render(&list(&state["inputs"]));
    render_volume(sp, true);
    render_volume(sp, false);
}

async fn gnome_sound(p: Page, state: Value) -> Content {
    let gs = Gs::load(&p, &[AMPLIFY]).await;
    if !p.alive() {
        return Content::new();
    }
    let sp = Rc::new(SoundPage {
        p: p.clone(),
        gs: gs.clone(),
        state: RefCell::new(state),
        outputs: DeviceList::new(&p, true),
        inputs: DeviceList::new(&p, false),
        out_slot: common::slot(),
        in_slot: common::slot(),
        out_vol: RefCell::new(None),
        in_vol: RefCell::new(None),
        meter: Meter::new(&p, "source", "st-sound-input-level"),
    });
    let output = SettingsGroup::new(t("Uitvoer", "Output"), "");
    output.add(sp.outputs.0.exp.widget());
    output.add(&sp.out_slot);
    let input = SettingsGroup::new(t("Invoer", "Input"), "");
    input.add(sp.inputs.0.exp.widget());
    input.add(&sp.in_slot);
    let advanced = SettingsGroup::new(t("Geavanceerd", "Advanced"), "");
    advanced.add(
        common::link_card_with(&p, "sound-devices", None, Some(t("Apparaten in-/uitschakelen, problemen oplossen, andere opties", "Turn devices on/off, troubleshoot, other options")), Some("devices"))
            .widget(),
    );
    advanced.add(common::link_card_with(&p, "volume-mixer", None, Some(t("App-volumemix, invoer- en uitvoerapparaten van apps", "App volume mix, app input & output devices")), Some("volume")).widget());
    let more = SettingsCard::clickable("sound", t("Meer geluidsinstellingen", "More sound settings"), "", Chevron::External);
    if let Some(b) = more.button() {
        b.set_widget_name("st-sound-more");
    }
    {
        let pp = p.clone();
        more.connect_clicked(move || more_sound_settings(&pp));
    }
    advanced.add(more.widget());
    render(&sp);
    {
        let sp2 = sp.clone();
        p.on("sound.changed", move |d| {
            if d["available"] != true {
                sp2.p.refresh();
                return;
            }
            *sp2.state.borrow_mut() = d.clone();
            render(&sp2);
        });
    }
    {
        let pp = p.clone();
        gs.follow(&AMPLIFY, move |_| pp.refresh());
    }
    vec![output.widget().clone().upcast(), input.widget().clone().upcast(), advanced.widget().clone().upcast()]
}

fn sound(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.get("sound.state", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if state["available"] == true {
            return gnome_sound(p, state).await;
        }
        // the page without a pulse server keeps its related link (glossary-context.tsv's sound/* row)
        let related =
            common::link_card_with(&p, "notifications", Some(t("Geluiden bij meldingen", "Notification sounds")), Some(t("Meldingsgeluiden aan of uit", "Turn notification sounds on or off")), None);
        legacy_sound(p, related).await
    })
}

// ------------------------------------------------------------------------------------------------ without pulse

struct LegacyState {
    state: RefCell<Value>,
    mute: gtk::Button,
    mute_glyph: gtk::Image,
    value: gtk::Label,
    slider: Slider,
    devices: gtk::Box,
    exp: SettingsExpander,
}

fn paint(st: &LegacyState) {
    let s = st.state.borrow();
    let volume = s["volume"].as_f64().unwrap_or(0.0);
    let muted = s["muted"] == true;
    kit::icons::set_glyph(&st.mute_glyph, kit::icons::volume_icon(volume, muted));
    st.mute.set_tooltip_text(Some(if muted { t("Dempen opheffen", "Unmute") } else { t("Dempen", "Mute") }));
    if muted {
        st.mute.add_css_class("st-muted");
    } else {
        st.mute.remove_css_class("st-muted");
    }
    st.value.set_label(&if muted { t("Gedempt", "Muted").to_owned() } else { format!("{}", volume.round() as i64) });
}

fn sinks(st: &LegacyState) -> Vec<Value> {
    st.state.borrow()["sinks"].as_array().cloned().unwrap_or_default()
}

fn update_desc(st: &LegacyState) {
    let list = sinks(st);
    let def = list.iter().find(|s| s["default"] == true);
    st.exp.header().set_description(match def {
        Some(d) => d["name"].as_str().unwrap_or(""),
        None => t("Apps gebruiken het apparaat dat je hier kiest", "Apps use the device you choose here"),
    });
}

fn render_devices(p: &Page, st: &Rc<LegacyState>) {
    c::clear(&st.devices);
    let list = sinks(st);
    if list.is_empty() {
        st.devices.append(&c::label(t("Geen geluidsapparaten gevonden", "No sound devices found"), "w11-text-2", true));
        return;
    }
    let def = list.iter().find(|s| s["default"] == true).or_else(|| list.first()).cloned().unwrap_or(Value::Null);
    let opts: Vec<(String, String)> = list.iter().map(|s| (common::value_key(&s["id"]), s["name"].as_str().unwrap_or("").to_owned())).collect();
    let refs: Vec<(&str, &str)> = opts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let def_key = common::value_key(&def["id"]);
    let group = RadioGroup::new(&refs, Some(&def_key), None, false);
    group.widget().set_widget_name("sound-output");
    let (pp, st2) = (p.clone(), st.clone());
    group.connect_changed(move |key| {
        let Some(sink) = sinks(&st2).into_iter().find(|s| common::value_key(&s["id"]) == key) else { return };
        let id = sink["id"].clone();
        let (pp, st2) = (pp.clone(), st2.clone());
        let fut = pp.run_with("system.audio.setSink", json!({ "id": id }), None, true);
        pp.clone().spawn(async move {
            let ok = fut.await.is_some();
            if !pp.alive() {
                return;
            }
            if ok {
                let list: Vec<Value> = sinks(&st2)
                    .into_iter()
                    .map(|mut s| {
                        let def = s["id"] == id;
                        s["default"] = json!(def);
                        s
                    })
                    .collect();
                st2.state.borrow_mut()["sinks"] = json!(list);
                update_desc(&st2);
            } else {
                render_devices(&pp, &st2);
            }
        });
    });
    st.devices.append(group.widget());
}

/// The page without a PulseAudio server (system.audio.*, wpctl): the output device, volume and mute.
async fn legacy_sound(p: Page, notification_sounds: SettingsCard) -> Content {
    let r = p.call("system.audio.get", json!({})).await;
    if !p.alive() {
        return Content::new();
    }
    let a = r.ok().unwrap_or(Value::Null);
    if a["available"] != true {
        let group = SettingsGroup::new(t("Uitvoer", "Output"), "");
        group.add(common::unavailable_card("sound", t("Kies waar je geluid wilt afspelen", "Choose where to play sound"), None).widget());
        group.add(common::unavailable_card("volume", t("Volume", "Volume"), None).widget());
        return vec![
            common::notice(t(
                "Het geluid kan hier niet worden geregeld. Gebruik de geluidsinstellingen van Ubuntu.",
                "Sound can’t be controlled here. Use Ubuntu’s sound settings.",
            ))
            .widget()
            .clone()
            .upcast(),
            group.widget().clone().upcast(),
        ];
    }

    let mute = gtk::Button::new();
    mute.add_css_class("w11-icon-button");
    mute.set_size_request(36, 36);
    mute.set_valign(gtk::Align::Center);
    mute.set_widget_name("sound-mute");
    let mute_glyph = kit::icons::glyph("volume", 18);
    mute.set_child(Some(&mute_glyph));
    let value = common::value_label("");
    value.add_css_class("w11-tabular");
    value.set_xalign(1.0);
    value.set_size_request(56, -1);
    let slider = Slider::new(0.0, 100.0, 1.0, a["volume"].as_f64().unwrap_or(0.0).round());
    slider.set_value_visible(false);
    slider.widget().set_size_request(240, -1);
    slider.scale().set_widget_name("sound-volume");
    let devices = gtk::Box::new(gtk::Orientation::Vertical, 4);
    devices.add_css_class("st-radio-list");
    let exp = SettingsExpander::new("sound", t("Kies waar je geluid wilt afspelen", "Choose where to play sound"), "");
    exp.add(&devices);
    exp.set_expanded(true);
    let st = Rc::new(LegacyState { state: RefCell::new(a.clone()), mute: mute.clone(), mute_glyph, value: value.clone(), slider: slider.clone(), devices, exp: exp.clone() });

    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    {
        let st2 = st.clone();
        slider.connect_changed(move |v| {
            st2.state.borrow_mut()["volume"] = json!(v.round());
            paint(&st2);
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let pend = pending.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(50), move || {
                pend.borrow_mut().take();
                kit::ctx().fire("system.audio.set", json!({ "volume": v.round() }));
            });
            *pending.borrow_mut() = Some(id);
        });
    }
    {
        let (pp, st2) = (p.clone(), st.clone());
        slider.connect_committed(move |v| {
            st2.state.borrow_mut()["volume"] = json!(v.round());
            paint(&st2);
            let fut = pp.run_with("system.audio.set", json!({ "volume": v.round() }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
    }
    {
        let (pp, st2) = (p.clone(), st.clone());
        mute.connect_clicked(move |_| {
            let muted = st2.state.borrow()["muted"] != true;
            st2.state.borrow_mut()["muted"] = json!(muted);
            paint(&st2);
            let fut = pp.run_with("system.audio.set", json!({ "muted": muted }), None, true);
            let st3 = st2.clone();
            pp.spawn(async move {
                if fut.await.is_none() {
                    st3.state.borrow_mut()["muted"] = json!(!muted);
                    paint(&st3);
                }
            });
        });
    }
    paint(&st);
    let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    vol_box.add_css_class("st-volume");
    vol_box.append(&mute);
    vol_box.append(slider.widget());
    vol_box.append(&value);
    let volume_card = SettingsCard::new("volume", t("Volume", "Volume"), t("Hoe hard het geluid is", "How loud the sound is"));
    volume_card.set_control(&vol_box);

    render_devices(&p, &st);
    update_desc(&st);
    {
        let (pp, st2) = (p.clone(), st.clone());
        p.on("system.audio.changed", move |d| {
            if !d.is_object() {
                return;
            }
            let changed = d["sinks"] != st2.state.borrow()["sinks"];
            if let (Some(cur), Some(new)) = (st2.state.borrow_mut().as_object_mut(), d.as_object()) {
                for (k, v) in new {
                    cur.insert(k.clone(), v.clone());
                }
            }
            st2.slider.set_value(st2.state.borrow()["volume"].as_f64().unwrap_or(0.0).round());
            paint(&st2);
            if changed {
                render_devices(&pp, &st2);
                update_desc(&st2);
            }
        });
    }

    let output = SettingsGroup::new(t("Uitvoer", "Output"), "");
    output.add(exp.widget());
    output.add(volume_card.widget());
    let related = SettingsGroup::new(ct::related(), "");
    related.add(notification_sounds.widget());
    vec![output.widget().clone().upcast(), related.widget().clone().upcast()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_default_follows_gnome_s_device_entry() {
        let speakers = json!({"id": "alsa_card.pci:analog-output-speaker", "kind": "sink", "device": "alsa_output.pci", "port": "analog-output-speaker", "card": "alsa_card.pci", "profile": null});
        assert_eq!(default_args(&speakers), json!({"kind": "sink", "name": "alsa_output.pci", "port": "analog-output-speaker", "card": "alsa_card.pci"}));
        let hdmi = json!({"id": "alsa_card.pci:hdmi-output-0", "kind": "sink", "device": null, "port": "hdmi-output-0", "card": "alsa_card.pci", "profile": "output:hdmi-stereo"});
        assert_eq!(default_args(&hdmi), json!({"kind": "sink", "port": "hdmi-output-0", "card": "alsa_card.pci", "profile": "output:hdmi-stereo"}), "a profile switch first");
        assert_eq!(props_params(&hdmi), json!({"kind": "sink", "id": "alsa_card.pci:hdmi-output-0", "card": "alsa_card.pci"}));
        assert_eq!(props_params(&json!({"kind": "sink", "id": "null-sink", "card": null})), json!({"kind": "sink", "id": "null-sink"}));
    }

    #[test]
    fn a_device_list_is_built_again_only_when_its_entries_change() {
        let a = vec![json!({"id": "x", "title": "Speakers", "subtitle": "Built-in Audio", "default": true})];
        let mut b = a.clone();
        b[0]["default"] = json!(false);
        assert_eq!(signature(&a), signature(&b), "the default moving is no new list");
        b[0]["title"] = json!("Headphones");
        assert_ne!(signature(&a), signature(&b));
    }

    #[test]
    fn writes_name_their_device_stream_or_the_event_role() {
        assert_eq!(Target::Device("sink", "a".into()).volume(40.0), ("sound.setVolume", json!({"kind": "sink", "name": "a", "volume": 40.0})));
        assert_eq!(Target::Stream("sinkInput", 7).mute(true, 30.0), ("sound.setMute", json!({"kind": "sinkInput", "index": 7, "mute": true})));
        assert_eq!(Target::Event.mute(true, 30.0), ("sound.setEventVolume", json!({"volume": 30.0, "mute": true})), "the event role keeps its volume");
        assert_eq!(device_glyph(&json!({"port": "analog-output-headphones"}), true), "headphones");
        assert_eq!(device_glyph(&json!({"port": "hdmi-output-0"}), true), "display");
        assert_eq!(device_glyph(&json!({}), false), "mic");
    }
}
