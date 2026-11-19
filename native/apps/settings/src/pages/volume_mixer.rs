//! Instellingen › Systeem › Geluid › Volumemixer (batch D, notes/claude/settings-parity.md §2) in Windows 11's layout
//! on GNOME's Volume Levels (host sound.*): Systeem (Uitvoerapparaat, Invoerapparaat, Volume), Apps (Systeemgeluiden =
//! GNOME's System Sounds, the event role; then every app's playback and recording with its volume and mute, and, as
//! Windows has per app, its Uitvoerapparaat or Invoerapparaat: sound.moveStream). Windows' "Geluidsapparaten en
//! volumes voor alle apps opnieuw instellen" has no GNOME function and is left out.
use super::sound::{self, Target, VolumeControl, AMPLIFY};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::Gs;
use kit::controls::{Dropdown, DropdownOption, SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("volume-mixer", volume_mixer)];
pub const CSS: &str = "";

/// Dropdown options of GNOME's device entries (id, title).
pub(super) fn entry_options(entries: &[Value]) -> Vec<DropdownOption> {
    entries.iter().map(|d| DropdownOption::new(&sound::s(&d["id"]), &sound::s(&d["title"]))).collect()
}

/// The devices a stream can move to: every sink (or source that is not a monitor), by name.
pub(super) fn move_targets(state: &Value, output: bool) -> Vec<(String, String)> {
    sound::list(&state[if output { "sinks" } else { "sources" }])
        .iter()
        .filter(|d| output || d["monitorOf"].is_null())
        .map(|d| (sound::s(&d["name"]), sound::s(&d["description"])))
        .collect()
}

/// The apps' streams' identity (a new or gone stream builds the Apps list again).
pub(super) fn streams_shape(state: &Value) -> String {
    sound::list(&state["streams"]).iter().map(|s| format!("{}:{}:{}", sound::s(&s["kind"]), s["index"], sound::s(&s["app"]))).collect::<Vec<_>>().join(",")
}

/// What the apps' Uitvoerapparaat and Invoerapparaat drop-downs offer: a sink or source that comes or goes while the
/// streams stay gives them their options again (review D-6: Bluetooth headphones connected while Spotify plays).
pub(super) fn targets_shape(state: &Value) -> String {
    serde_json::to_string(&(move_targets(state, true), move_targets(state, false))).unwrap_or_default()
}

fn target_options(state: &Value, output: bool) -> Vec<DropdownOption> {
    move_targets(state, output).iter().map(|(n, d)| DropdownOption::new(n, d)).collect()
}

struct StreamRow {
    vc: VolumeControl,
    device: Dropdown,
    output: bool,
}

struct Mixer {
    p: Page,
    gs: Gs,
    state: RefCell<Value>,
    output: Dropdown,
    input: Dropdown,
    master_slot: gtk::Box,
    master: RefCell<Option<(String, VolumeControl)>>,
    event: RefCell<Option<VolumeControl>>,
    apps: gtk::Box,
    shape: RefCell<String>,
    targets: RefCell<String>,
    rows: RefCell<HashMap<String, StreamRow>>,
}

fn choose_default(m: &Rc<Mixer>, output: bool, id: &str) {
    let state = m.state.borrow().clone();
    let Some(entry) = sound::list(&state[if output { "outputs" } else { "inputs" }]).into_iter().find(|d| d["id"] == id) else { return };
    let wait = m.p.run_with("sound.setDefault", sound::default_args(&entry), Some(t("Het geluidsapparaat kon niet worden gewijzigd", "Couldn’t change the sound device")), true);
    let m2 = m.clone();
    m.p.spawn(async move {
        if wait.await.is_none() && m2.p.alive() {
            render_system(&m2);
        }
    });
}

fn render_system(m: &Rc<Mixer>) {
    let state = m.state.borrow().clone();
    for (output, dd) in [(true, &m.output), (false, &m.input)] {
        let entries = sound::list(&state[if output { "outputs" } else { "inputs" }]);
        dd.set_options(entry_options(&entries));
        dd.set_selected(entries.iter().find(|d| d["default"] == true).and_then(|d| d["id"].as_str()));
        dd.widget().set_sensitive(!entries.is_empty());
    }
    let name = sound::s(&state["defaultSink"]);
    match sound::device(&state, true, &name) {
        Some(dev) => {
            let (v, muted) = (dev["volume"].as_f64().unwrap_or(0.0), dev["mute"] == true);
            if let Some((current, vc)) = m.master.borrow().as_ref()
                && *current == name
            {
                vc.update(v, muted);
                return;
            }
            let vc = VolumeControl::new(&m.p, "st-volume-mixer-volume", false, v, muted, sound::max_volume(&m.gs, &state), Target::Device("sink", name.clone()), true);
            common::set_slot(&m.master_slot, &[vc.widget().clone().upcast()]);
            *m.master.borrow_mut() = Some((name, vc));
        }
        None => {
            common::set_slot(&m.master_slot, &[]);
            *m.master.borrow_mut() = None;
        }
    }
}

fn render_apps(m: &Rc<Mixer>) {
    let state = m.state.borrow().clone();
    if let Some(vc) = m.event.borrow().as_ref() {
        vc.update(state["eventVolume"].as_f64().unwrap_or(100.0), state["eventMute"] == true);
    }
    let shape = streams_shape(&state);
    let streams = sound::list(&state["streams"]);
    let targets = targets_shape(&state);
    if *m.shape.borrow() == shape {
        let new_targets = *m.targets.borrow() != targets;
        *m.targets.borrow_mut() = targets;
        let rows = m.rows.borrow();
        for s in &streams {
            if let Some(row) = rows.get(&format!("{}:{}", sound::s(&s["kind"]), s["index"])) {
                row.vc.update(s["volume"].as_f64().unwrap_or(0.0), s["mute"] == true);
                if new_targets {
                    row.device.set_options(target_options(&state, row.output));
                }
                row.device.set_selected(s["device"].as_str());
            }
        }
        return;
    }
    *m.shape.borrow_mut() = shape;
    *m.targets.borrow_mut() = targets;
    let mut rows = HashMap::new();
    let mut widgets: Vec<gtk::Widget> = Vec::new();
    if streams.is_empty() {
        let none = SettingsCard::new("", t("Er spelen nu geen apps geluid af en er nemen geen apps op", "No apps are playing or recording sound right now"), "");
        none.widget().set_widget_name("st-volume-mixer-no-apps");
        widgets.push(none.widget().clone());
    }
    let max = sound::max_volume(&m.gs, &state);
    for s in streams {
        let output = s["kind"] == "output";
        let index = s["index"].as_u64().unwrap_or(0);
        let kind = if output { "sinkInput" } else { "sourceOutput" };
        let icon = s["icon"].as_str().filter(|i| !i.is_empty()).unwrap_or(if output { "app" } else { "mic" }).to_owned();
        let app = sound::s(&s["app"]);
        let name = sound::s(&s["name"]);
        let desc = if output {
            if name != app { name.clone() } else { String::new() }
        } else {
            t("Neemt op", "Recording").to_owned()
        };
        // the app's card with its volume and mute, its device on the row below: not an expander, whose header button
        // would take the pointer's clicks on the mute button (kit cards.rs)
        let app_card = SettingsCard::new(&icon, &app, &desc);
        app_card.widget().set_widget_name(&format!("st-volume-mixer-app-{index}"));
        let vc = VolumeControl::new(&m.p, &format!("st-volume-mixer-app-{index}-volume"), !output, s["volume"].as_f64().unwrap_or(0.0), s["mute"] == true, max, Target::Stream(kind, index), s["volumeWritable"] != false);
        app_card.set_control(vc.widget());
        let dd = Dropdown::new(target_options(&state, output), s["device"].as_str());
        dd.widget().set_widget_name(&format!("st-volume-mixer-app-{index}-device"));
        {
            let m2 = m.clone();
            dd.connect_changed(move |device| {
                let wait = m2.p.run_with("sound.moveStream", json!({"kind": kind, "index": index, "device": device}), Some(t("Het apparaat van deze app kon niet worden gewijzigd", "Couldn’t change this app’s device")), true);
                let m3 = m2.clone();
                m2.p.spawn(async move {
                    if wait.await.is_none() && m3.p.alive() {
                        m3.shape.borrow_mut().clear();
                        render_apps(&m3);
                    }
                });
            });
        }
        let row = sub_row(SettingsCard::new("", if output { t("Uitvoerapparaat", "Output device") } else { t("Invoerapparaat", "Input device") }, ""));
        row.set_control(dd.widget());
        let pair = gtk::Box::new(gtk::Orientation::Vertical, 4);
        pair.append(app_card.widget());
        pair.append(row.widget());
        widgets.push(pair.upcast());
        rows.insert(format!("{}:{}", sound::s(&s["kind"]), index), StreamRow { vc, device: dd, output });
    }
    *m.rows.borrow_mut() = rows;
    common::set_slot(&m.apps, &widgets);
}

fn volume_mixer(p: Page) -> PageFuture {
    Box::pin(async move {
        let state = p.call("sound.state", json!({}));
        let gs = Gs::load(&p, &[AMPLIFY]).await;
        let state = state.await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if state["available"] != true {
            let bar = common::notice(t("Geluidsapparaten kunnen hier niet worden geregeld.", "Sound devices can’t be controlled here."));
            let group = SettingsGroup::new("", "");
            group.add(common::link_card(&p, "sound").widget());
            return vec![bar.widget().clone().upcast(), group.widget().clone().upcast()];
        }
        let output = Dropdown::new(Vec::new(), None);
        output.widget().set_widget_name("st-volume-mixer-output");
        let input = Dropdown::new(Vec::new(), None);
        input.widget().set_widget_name("st-volume-mixer-input");
        let m = Rc::new(Mixer {
            p: p.clone(),
            gs: gs.clone(),
            state: RefCell::new(state.clone()),
            output: output.clone(),
            input: input.clone(),
            master_slot: common::slot(),
            master: RefCell::new(None),
            event: RefCell::new(None),
            apps: common::slot(),
            shape: RefCell::new(String::from("\u{0}")),
            targets: RefCell::new(String::new()),
            rows: RefCell::new(HashMap::new()),
        });
        {
            let m2 = m.clone();
            output.connect_changed(move |id| choose_default(&m2, true, id));
            let m2 = m.clone();
            input.connect_changed(move |id| choose_default(&m2, false, id));
        }
        let system = SettingsGroup::new(t("Systeem", "System"), "");
        let row = SettingsCard::new("sound", t("Uitvoerapparaat", "Output device"), "");
        row.set_control(output.widget());
        system.add(row.widget());
        let row = SettingsCard::new("mic", t("Invoerapparaat", "Input device"), "");
        row.set_control(input.widget());
        system.add(row.widget());
        let row = SettingsCard::new("volume", t("Volume", "Volume"), "");
        row.set_control(&m.master_slot);
        system.add(row.widget());
        let apps = SettingsGroup::new(t("Apps", "Apps"), "");
        let event = VolumeControl::new(&p, "st-volume-mixer-event", false, state["eventVolume"].as_f64().unwrap_or(100.0), state["eventMute"] == true, 100.0, Target::Event, true);
        let row = SettingsCard::new("notification", t("Systeemgeluiden", "System sounds"), "");
        row.set_control(event.widget());
        apps.add(row.widget());
        apps.add(&m.apps);
        *m.event.borrow_mut() = Some(event);
        render_system(&m);
        render_apps(&m);
        {
            let m2 = m.clone();
            p.on("sound.changed", move |d| {
                if d["available"] != true {
                    m2.p.refresh();
                    return;
                }
                *m2.state.borrow_mut() = d.clone();
                render_system(&m2);
                render_apps(&m2);
            });
        }
        {
            let pp = p.clone();
            gs.follow(&AMPLIFY, move |_| pp.refresh());
        }
        vec![system.widget().clone().upcast(), apps.widget().clone().upcast()]
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
mod tests {
    use super::*;

    #[test]
    fn streams_move_to_real_devices_only() {
        let state = json!({
            "sinks": [{"name": "a", "description": "Speakers"}],
            "sources": [{"name": "a.monitor", "description": "Monitor", "monitorOf": "a"}, {"name": "mic", "description": "Microphone", "monitorOf": null}],
            "streams": [{"kind": "output", "index": 4, "app": "Firefox"}, {"kind": "input", "index": 9, "app": "Recorder"}]
        });
        assert_eq!(move_targets(&state, true), vec![("a".to_owned(), "Speakers".to_owned())]);
        assert_eq!(move_targets(&state, false), vec![("mic".to_owned(), "Microphone".to_owned())], "never a monitor");
        assert_eq!(streams_shape(&state), "output:4:Firefox,input:9:Recorder");
        let mut louder = state.clone();
        louder["streams"][0]["volume"] = json!(20);
        assert_eq!(streams_shape(&louder), streams_shape(&state), "a volume change is no new list");
        assert_eq!(targets_shape(&louder), targets_shape(&state), "nor new device options");
        // review D-6: headphones connect while the streams stay; the apps' drop-downs get them
        let mut headphones = state.clone();
        headphones["sinks"].as_array_mut().unwrap().push(json!({"name": "bluez_output.a", "description": "Headphones"}));
        assert_eq!(streams_shape(&headphones), streams_shape(&state));
        assert_ne!(targets_shape(&headphones), targets_shape(&state), "a new sink sets the options again");
        assert_eq!(move_targets(&headphones, true)[1], ("bluez_output.a".to_owned(), "Headphones".to_owned()));
        let mut mic = state.clone();
        mic["sources"].as_array_mut().unwrap().push(json!({"name": "bluez_input.a", "description": "Headset", "monitorOf": null}));
        assert_ne!(targets_shape(&mic), targets_shape(&state), "and so does a new source");
    }
}
