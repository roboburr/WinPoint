//! Instellingen › Systeem › Geluid (vendor/ui/js/apps/settings/pages/sound.js): the output device, volume
//! and mute.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use kit::controls::{self as c, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, Slider};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("sound", sound)];
pub const CSS: &str = "";

struct State {
    state: RefCell<Value>,
    mute: gtk::Button,
    mute_glyph: gtk::Image,
    value: gtk::Label,
    slider: Slider,
    devices: gtk::Box,
    exp: SettingsExpander,
}

fn paint(st: &State) {
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

fn sinks(st: &State) -> Vec<Value> {
    st.state.borrow()["sinks"].as_array().cloned().unwrap_or_default()
}

fn update_desc(st: &State) {
    let list = sinks(st);
    let def = list.iter().find(|s| s["default"] == true);
    st.exp.header().set_description(match def {
        Some(d) => d["name"].as_str().unwrap_or(""),
        None => t("Apps gebruiken het apparaat dat je hier kiest", "Apps use the device you choose here"),
    });
}

fn render_devices(p: &Page, st: &Rc<State>) {
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

fn sound(p: Page) -> PageFuture {
    Box::pin(async move {
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

        // ---- volume and mute
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
        let st = Rc::new(State { state: RefCell::new(a.clone()), mute: mute.clone(), mute_glyph, value: value.clone(), slider: slider.clone(), devices, exp: exp.clone() });

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
                let id = glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
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
        related.add(
            common::link_card_with(&p, "notifications", Some(t("Geluiden bij meldingen", "Notification sounds")), Some(t("Meldingsgeluiden aan of uit", "Turn notification sounds on or off")), None)
                .widget(),
        );
        vec![output.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}
