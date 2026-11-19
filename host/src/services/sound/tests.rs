//! Batch D's sound tests (notes/claude/settings-parity.md §7; plan "testing": pulse volume and balance math). No
//! test reaches a PulseAudio server: client.rs connects in a test build only to WIN11_TEST_PULSE_SERVER, which no
//! test sets, so these cover the layouts, the math through libpulse's own pure functions, GNOME's device lists and
//! the streams on fixtures, system.audio's payload, and the alert sounds on a memory GSettings backend and a
//! temporary data directory. The harness scenario (tests/gnome/scenarios/settings-system.py) runs the pages on a
//! private PipeWire.
use super::model::{
    self, Card, CardPort, Client as PaClient, Device, Model, Port, Profile, Server, Stream,
};
use super::pulse::{self, CVolume, ChannelMap};
use super::{Target, alerts, client, legacy_state, target, volume_arg};
use crate::files::ApiError;
use crate::i18n::Lang;
use crate::services::gsettings;
use serde_json::{Value, json};
use std::mem::{offset_of, size_of};

// ------------------------------------------------------------------------------------------------ layouts

/// The sizes and offsets of libpulse 17 on x86_64, as `offsetof()` printed them (a C program against
/// /usr/include/pulse; pulse.rs module docs).
#[test]
fn the_libpulse_layouts_match_the_c_headers() {
    assert_eq!(size_of::<pulse::SampleSpec>(), 12);
    assert_eq!(size_of::<ChannelMap>(), 132);
    assert_eq!(size_of::<CVolume>(), 132);
    assert_eq!(size_of::<pulse::BufferAttr>(), 20);
    assert_eq!(size_of::<pulse::ServerInfo>(), 200);
    assert_eq!(offset_of!(pulse::ServerInfo, default_sink_name), 48);
    assert_eq!(offset_of!(pulse::ServerInfo, default_source_name), 56);
    assert_eq!(offset_of!(pulse::ServerInfo, channel_map), 68);
    assert_eq!(size_of::<pulse::PortInfo>(), 40);
    assert_eq!(offset_of!(pulse::PortInfo, available), 20);
    assert_eq!(offset_of!(pulse::PortInfo, type_), 32);
    type D = pulse::DeviceInfo;
    assert_eq!(size_of::<D>(), 416);
    for (offset, expected) in [
        (offset_of!(D, index), 8),
        (offset_of!(D, description), 16),
        (offset_of!(D, sample_spec), 24),
        (offset_of!(D, channel_map), 36),
        (offset_of!(D, owner_module), 168),
        (offset_of!(D, volume), 172),
        (offset_of!(D, mute), 304),
        (offset_of!(D, monitor), 308),
        (offset_of!(D, monitor_name), 312),
        (offset_of!(D, latency), 320),
        (offset_of!(D, driver), 328),
        (offset_of!(D, flags), 336),
        (offset_of!(D, proplist), 344),
        (offset_of!(D, configured_latency), 352),
        (offset_of!(D, base_volume), 360),
        (offset_of!(D, state), 364),
        (offset_of!(D, n_volume_steps), 368),
        (offset_of!(D, card), 372),
        (offset_of!(D, n_ports), 376),
        (offset_of!(D, ports), 384),
        (offset_of!(D, active_port), 392),
        (offset_of!(D, n_formats), 400),
        (offset_of!(D, formats), 408),
    ] {
        assert_eq!(offset, expected);
    }
    type I = pulse::SinkInputInfo;
    assert_eq!(size_of::<I>(), 376);
    for (offset, expected) in [
        (offset_of!(I, client), 20),
        (offset_of!(I, sink), 24),
        (offset_of!(I, sample_spec), 28),
        (offset_of!(I, channel_map), 40),
        (offset_of!(I, volume), 172),
        (offset_of!(I, buffer_usec), 304),
        (offset_of!(I, resample_method), 320),
        (offset_of!(I, driver), 328),
        (offset_of!(I, mute), 336),
        (offset_of!(I, proplist), 344),
        (offset_of!(I, corked), 352),
        (offset_of!(I, has_volume), 356),
        (offset_of!(I, volume_writable), 360),
        (offset_of!(I, format), 368),
    ] {
        assert_eq!(offset, expected);
    }
    type O = pulse::SourceOutputInfo;
    assert_eq!(size_of::<O>(), 376);
    for (offset, expected) in [
        (offset_of!(O, source), 24),
        (offset_of!(O, channel_map), 40),
        (offset_of!(O, buffer_usec), 176),
        (offset_of!(O, resample_method), 192),
        (offset_of!(O, driver), 200),
        (offset_of!(O, proplist), 208),
        (offset_of!(O, corked), 216),
        (offset_of!(O, volume), 220),
        (offset_of!(O, mute), 352),
        (offset_of!(O, has_volume), 356),
        (offset_of!(O, volume_writable), 360),
        (offset_of!(O, format), 368),
    ] {
        assert_eq!(offset, expected);
    }
    assert_eq!(size_of::<pulse::ClientInfo>(), 40);
    assert_eq!(offset_of!(pulse::ClientInfo, driver), 24);
    assert_eq!(offset_of!(pulse::ClientInfo, proplist), 32);
    assert_eq!(size_of::<pulse::CardProfileInfo2>(), 32);
    assert_eq!(offset_of!(pulse::CardProfileInfo2, n_sinks), 16);
    assert_eq!(offset_of!(pulse::CardProfileInfo2, priority), 24);
    assert_eq!(offset_of!(pulse::CardProfileInfo2, available), 28);
    type P = pulse::CardPortInfo;
    assert_eq!(size_of::<P>(), 80);
    for (offset, expected) in [
        (offset_of!(P, priority), 16),
        (offset_of!(P, available), 20),
        (offset_of!(P, direction), 24),
        (offset_of!(P, n_profiles), 28),
        (offset_of!(P, profiles), 32),
        (offset_of!(P, proplist), 40),
        (offset_of!(P, latency_offset), 48),
        (offset_of!(P, profiles2), 56),
        (offset_of!(P, availability_group), 64),
        (offset_of!(P, type_), 72),
    ] {
        assert_eq!(offset, expected);
    }
    type C = pulse::CardInfo;
    assert_eq!(size_of::<C>(), 96);
    for (offset, expected) in [
        (offset_of!(C, owner_module), 16),
        (offset_of!(C, driver), 24),
        (offset_of!(C, n_profiles), 32),
        (offset_of!(C, profiles), 40),
        (offset_of!(C, active_profile), 48),
        (offset_of!(C, proplist), 56),
        (offset_of!(C, n_ports), 64),
        (offset_of!(C, ports), 72),
        (offset_of!(C, profiles2), 80),
        (offset_of!(C, active_profile2), 88),
    ] {
        assert_eq!(offset, expected);
    }
    type R = pulse::ExtStreamRestoreInfo;
    assert_eq!(size_of::<R>(), 288);
    assert_eq!(offset_of!(R, channel_map), 8);
    assert_eq!(offset_of!(R, volume), 140);
    assert_eq!(offset_of!(R, device), 272);
    assert_eq!(offset_of!(R, mute), 280);
}

// ------------------------------------------------------------------------------------------------ math

#[test]
fn volumes_are_percent_of_pa_volume_norm_and_the_loudest_channel_counts() {
    assert_eq!(model::pct(pulse::VOLUME_NORM), 100.0);
    assert_eq!(model::from_pct(100.0), pulse::VOLUME_NORM);
    assert_eq!(model::from_pct(50.0), 32768);
    assert_eq!(model::from_pct(-3.0), 0);
    assert_eq!(model::ui_pct(&[32768, 65536]), 100, "gvc: pa_cvolume_max");
    assert_eq!(model::ui_pct(&[]), 0);
    // Linkerkanaal / Rechterkanaal <-> volume and balance (pa_cvolume_get_balance's formula)
    assert_eq!(model::volume_balance(50.0, 100.0), (100.0, 0.5));
    assert_eq!(model::volume_balance(80.0, 20.0), (80.0, -0.75));
    assert_eq!(model::volume_balance(40.0, 40.0), (40.0, 0.0));
    assert_eq!(model::volume_balance(0.0, 0.0), (0.0, 0.0));
    for (l, r) in [(50.0, 100.0), (80.0, 20.0), (40.0, 40.0), (0.0, 30.0)] {
        let (v, b) = model::volume_balance(l, r);
        let (l2, r2) = model::left_right(v, b);
        assert!(
            (l2 - l).abs() < 1e-9 && (r2 - r).abs() < 1e-9,
            "{l} {r} -> {v} {b} -> {l2} {r2}"
        );
    }
    assert_eq!(
        volume_arg(&json!(120), 100.0),
        Some(100.0),
        "above 100 % only with allow-volume-above-100-percent"
    );
    assert_eq!(volume_arg(&json!(120), 152.6), Some(120.0));
    assert_eq!(volume_arg(&json!(-1), 100.0), None);
    assert_eq!(volume_arg(&json!("50"), 100.0), None);
    assert_eq!(target(&json!({"kind": "sink"})), Some(Target::Sink));
    assert_eq!(
        target(&json!({"kind": "sourceOutput"})),
        Some(Target::SourceOutput)
    );
    assert_eq!(target(&json!({"kind": "speakers"})), None);
}

/// GNOME's balance, fade and subwoofer go through libpulse's own pure functions (no server involved).
#[test]
fn balance_fade_and_subwoofer_use_libpulse_as_gnome_does() {
    let lib = &pulse::libs()
        .expect("libpulse.so.0 and libpulse-mainloop-glib.so.0 load")
        .pa;
    const FL: i32 = 1;
    const FR: i32 = 2;
    const RL: i32 = 5;
    const RR: i32 = 6;
    const LFE: i32 = 7;
    let map = ChannelMap::of(&[FL, FR]);
    let mut cv = CVolume::of(&[pulse::VOLUME_NORM, pulse::VOLUME_NORM]);
    unsafe {
        assert_ne!((lib.pa_channel_map_can_balance)(&map), 0);
        assert_eq!((lib.pa_channel_map_can_fade)(&map), 0, "stereo has no rear");
        (lib.pa_cvolume_set_balance)(&mut cv, &map, 0.5);
        assert_eq!(cv.values(), vec![32768, 65536], "balance 0.5: left at half");
        assert!(((lib.pa_cvolume_get_balance)(&cv, &map) - 0.5).abs() < 1e-3);
        (lib.pa_cvolume_scale)(&mut cv, 32768);
        assert_eq!(
            cv.values(),
            vec![16384, 32768],
            "pa_cvolume_scale keeps the balance"
        );
        assert_eq!((lib.pa_cvolume_max)(&cv), 32768);
    }
    let surround = ChannelMap::of(&[FL, FR, RL, RR, LFE]);
    let mut cv = CVolume::of(&[pulse::VOLUME_NORM; 5]);
    unsafe {
        assert_ne!((lib.pa_channel_map_can_fade)(&surround), 0);
        assert_ne!(
            (lib.pa_channel_map_has_position)(&surround, pulse::POSITION_LFE),
            0
        );
        (lib.pa_cvolume_set_fade)(&mut cv, &surround, -0.5);
        let v = cv.values();
        assert_eq!(
            (v[0], v[2]),
            (32768, 65536),
            "fade -0.5: the front at half (negative is towards the rear)"
        );
        (lib.pa_cvolume_set_position)(&mut cv, &surround, pulse::POSITION_LFE, 13107);
        assert_eq!(cv.values()[4], 13107);
        assert_eq!(
            pulse::text((lib.pa_channel_position_to_string)(FL)),
            "front-left"
        );
        assert_eq!(pulse::text((lib.pa_channel_position_to_string)(LFE)), "lfe");
        // GNOME's amplified maximum (PA_VOLUME_UI_MAX, +11 dB)
        assert_eq!((lib.pa_sw_volume_from_dB)(11.0), 99957);
    }
}

// ------------------------------------------------------------------------------------------------ fixtures

fn props(pairs: &[(&str, &str)]) -> model::Props {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn port(name: &str, description: &str, available: &'static str) -> Port {
    Port {
        name: name.into(),
        description: description.into(),
        priority: 100,
        available,
    }
}

fn profile(name: &str, sinks: u32, sources: u32, priority: u32) -> Profile {
    Profile {
        name: name.into(),
        description: name.into(),
        sinks,
        sources,
        priority,
        available: true,
    }
}

/// A laptop: one card with analog speakers and headphones (unplugged), HDMI on another profile, a microphone;
/// a Bluetooth sink without ports on its own card is left out for simplicity; a null sink without a card.
fn laptop() -> Model {
    let mut m = Model::default();
    m.server = Server {
        name: "PulseAudio (on PipeWire 1.6.2)".into(),
        version: "15.0.0".into(),
        default_sink: "alsa_output.pci.analog-stereo".into(),
        default_source: "alsa_input.pci.analog-stereo".into(),
    };
    m.cards.insert(
        40,
        Card {
            index: 40,
            name: "alsa_card.pci".into(),
            driver: "alsa".into(),
            profiles: vec![
                profile("off", 0, 0, 0),
                profile("output:analog-stereo+input:analog-stereo", 1, 1, 6565),
                profile("output:hdmi-stereo", 1, 0, 5900),
                profile("output:hdmi-stereo+input:analog-stereo", 1, 1, 5965),
            ],
            active_profile: Some("output:analog-stereo+input:analog-stereo".into()),
            ports: vec![
                CardPort {
                    name: "analog-output-speaker".into(),
                    description: "Speakers".into(),
                    priority: 10000,
                    available: "unknown",
                    output: true,
                    profiles: vec!["output:analog-stereo+input:analog-stereo".into()],
                },
                CardPort {
                    name: "analog-output-headphones".into(),
                    description: "Headphones".into(),
                    priority: 9900,
                    available: "no",
                    output: true,
                    profiles: vec!["output:analog-stereo+input:analog-stereo".into()],
                },
                CardPort {
                    name: "hdmi-output-0".into(),
                    description: "HDMI / DisplayPort".into(),
                    priority: 5900,
                    available: "yes",
                    output: true,
                    profiles: vec![
                        "output:hdmi-stereo".into(),
                        "output:hdmi-stereo+input:analog-stereo".into(),
                    ],
                },
                CardPort {
                    name: "analog-input-mic".into(),
                    description: "Microphone".into(),
                    priority: 8700,
                    available: "unknown",
                    output: false,
                    profiles: vec![
                        "output:analog-stereo+input:analog-stereo".into(),
                        "output:hdmi-stereo+input:analog-stereo".into(),
                    ],
                },
            ],
            props: props(&[
                ("device.description", "Built-in Audio"),
                ("device.icon_name", "audio-card-analog-pci"),
                ("device.bus", "pci"),
            ]),
        },
    );
    m.sinks.insert(
        52,
        Device {
            index: 52,
            name: "alsa_output.pci.analog-stereo".into(),
            description: "Built-in Audio Analog Stereo".into(),
            card: Some(40),
            ports: vec![
                port("analog-output-speaker", "Speakers", "unknown"),
                port("analog-output-headphones", "Headphones", "no"),
            ],
            active_port: Some("analog-output-speaker".into()),
            volume: vec![32768, 65536],
            map: vec![1, 2],
            mute: false,
            base_volume: pulse::VOLUME_NORM,
            monitor_name: Some("alsa_output.pci.analog-stereo.monitor".into()),
            props: props(&[("object.id", "48"), ("device.icon_name", "audio-speakers")]),
            ..Default::default()
        },
    );
    m.sinks.insert(
        60,
        Device {
            index: 60,
            name: "null-sink".into(),
            description: "Dummy Output".into(),
            volume: vec![pulse::VOLUME_NORM],
            map: vec![0],
            mute: true,
            ..Default::default()
        },
    );
    m.sources.insert(
        53,
        Device {
            index: 53,
            name: "alsa_output.pci.analog-stereo.monitor".into(),
            description: "Monitor of Built-in Audio".into(),
            card: Some(40),
            monitor_of: Some(52),
            volume: vec![pulse::VOLUME_NORM; 2],
            map: vec![1, 2],
            ..Default::default()
        },
    );
    m.sources.insert(
        54,
        Device {
            index: 54,
            name: "alsa_input.pci.analog-stereo".into(),
            description: "Built-in Audio Analog Stereo".into(),
            card: Some(40),
            ports: vec![port("analog-input-mic", "Microphone", "unknown")],
            active_port: Some("analog-input-mic".into()),
            volume: vec![45875, 45875],
            map: vec![1, 2],
            ..Default::default()
        },
    );
    m.clients.insert(
        7,
        PaClient {
            index: 7,
            name: "Firefox".into(),
            props: props(&[
                ("application.name", "Firefox"),
                ("application.icon_name", "firefox"),
            ]),
        },
    );
    m.clients.insert(
        8,
        PaClient {
            index: 8,
            name: "WinPoint".into(),
            props: props(&[("application.id", "win11-gnome")]),
        },
    );
    let stream = |index: u32, client: u32, device: u32, p: model::Props| Stream {
        index,
        name: "Playback".into(),
        client: Some(client),
        device,
        volume: vec![pulse::VOLUME_NORM; 2],
        map: vec![1, 2],
        has_volume: true,
        volume_writable: true,
        props: p,
        ..Default::default()
    };
    m.sink_inputs
        .insert(70, stream(70, 7, 52, props(&[("media.name", "Video")])));
    m.sink_inputs.insert(
        71,
        stream(
            71,
            8,
            52,
            props(&[("media.role", "event"), ("application.name", "WinPoint")]),
        ),
    );
    m.source_outputs.insert(
        80,
        stream(
            80,
            8,
            54,
            props(&[("application.id", client::METER_APP_ID)]),
        ),
    );
    m.source_outputs.insert(
        81,
        stream(81, 7, 53, props(&[("application.name", "Recorder")])),
    );
    m.source_outputs.insert(
        82,
        stream(82, 7, 54, props(&[("application.name", "Firefox")])),
    );
    m
}

#[test]
fn gnome_device_lists_have_every_available_port_and_the_profile_a_port_needs() {
    let m = laptop();
    let outputs = model::devices(&m, true);
    let ids: Vec<&str> = outputs.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "alsa_card.pci:analog-output-speaker",
            "alsa_card.pci:hdmi-output-0",
            "null-sink"
        ],
        "unplugged headphones are left out"
    );
    let speakers = &outputs[0];
    assert_eq!(speakers["title"], "Speakers");
    assert_eq!(speakers["subtitle"], "Built-in Audio");
    assert_eq!(speakers["device"], "alsa_output.pci.analog-stereo");
    assert_eq!(speakers["default"], true);
    assert_eq!(speakers["profile"], Value::Null);
    let hdmi = &outputs[1];
    assert_eq!(hdmi["device"], Value::Null, "not in the active profile");
    assert_eq!(
        hdmi["profile"], "output:hdmi-stereo+input:analog-stereo",
        "keeps the microphone (gvc's best profile)"
    );
    assert_eq!(hdmi["default"], false);
    assert_eq!(outputs[2]["title"], "Dummy Output");
    let inputs = model::devices(&m, false);
    assert_eq!(inputs.len(), 1, "monitors are never inputs");
    assert_eq!(inputs[0]["title"], "Microphone");
    assert_eq!(inputs[0]["default"], true);
    let card = &m.cards[&40];
    assert_eq!(
        model::best_profile(card, None),
        Some("output:analog-stereo+input:analog-stereo"),
        "Toestaan without a remembered profile"
    );
}

#[test]
fn the_state_lists_apps_without_meters_event_sounds_or_monitor_recordings() {
    let mut m = laptop();
    m.cards.get_mut(&40).unwrap().active_profile = Some("off".into());
    m.previous_profiles
        .insert("alsa_card.pci".into(), "output:hdmi-stereo".into());
    let lib = pulse::libs().map(|l| &l.pa);
    let state = m.state_json(lib, 152.5);
    assert_eq!(state["available"], true);
    assert_eq!(state["maxVolume"], 153.0);
    let streams = state["streams"].as_array().unwrap();
    let listed: Vec<(i64, &str)> = streams
        .iter()
        .map(|s| (s["index"].as_i64().unwrap(), s["app"].as_str().unwrap()))
        .collect();
    assert_eq!(
        listed,
        [(70, "Firefox"), (82, "Firefox")],
        "event role, our meter and a monitor recording are not listed"
    );
    assert_eq!(streams[0]["icon"], "firefox");
    assert_eq!(streams[0]["device"], "alsa_output.pci.analog-stereo");
    assert_eq!(streams[0]["kind"], "output");
    let card = &state["cards"][0];
    assert_eq!(card["off"], true);
    assert_eq!(
        card["restore"], "output:hdmi-stereo",
        "the profile before Niet toestaan"
    );
    assert_eq!(card["outputs"], true);
    assert_eq!(card["inputs"], true);
    let sink = state["sinks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["index"] == 52)
        .unwrap()
        .clone();
    assert_eq!(sink["volume"], 100);
    assert_eq!(sink["monitor"], "alsa_output.pci.analog-stereo.monitor");
    assert_eq!(sink["objectId"], 48);
    if lib.is_some() {
        assert_eq!(sink["canBalance"], true);
        assert_eq!(sink["balance"], 0.5);
        assert_eq!(sink["channels"][0]["position"], "front-left");
        assert_eq!(sink["channels"][0]["volume"], 50.0);
    }
    let monitor = state["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["index"] == 53)
        .unwrap()
        .clone();
    assert_eq!(monitor["monitorOf"], "alsa_output.pci.analog-stereo");
    assert_eq!(
        state["eventVolume"], 100,
        "no stream-restore entry: GNOME's 100 %"
    );
}

#[test]
fn system_audio_keeps_its_payload_and_wpctl_ids_on_pulse() {
    let m = laptop();
    let s = legacy_state(&m);
    assert_eq!(s["available"], true);
    assert_eq!(s["volume"], 100);
    assert_eq!(s["muted"], false);
    assert_eq!(
        s["sinks"],
        json!([{"id": 48, "name": "Built-in Audio Analog Stereo", "default": true}, {"id": 60, "name": "Dummy Output", "default": false}]),
        "PipeWire's object id where pulse has it"
    );
    let mut none = Model::default();
    none.server.default_sink = "gone".into();
    assert_eq!(legacy_state(&none)["available"], false);
}

#[test]
fn a_test_build_never_connects_to_a_real_server() {
    if std::env::var_os("WIN11_TEST_PULSE_SERVER").is_none() {
        assert!(!client::possible());
        assert!(client::shared().is_none());
        assert!(super::audio_state().is_none(), "system.audio keeps wpctl");
        assert!(!super::audio_ready());
    }
}

// ------------------------------------------------------------------------------------------------ alerts

struct MemKeys(gsettings::Env);

impl alerts::Keys for MemKeys {
    fn read(&mut self, schema: &str, key: &str, lang: Lang) -> Value {
        gsettings::read_item_in(&mut self.0, schema, None, key, lang).unwrap_or(Value::Null)
    }

    fn write(
        &mut self,
        schema: &str,
        values: &[(&str, Value)],
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError> {
        gsettings::write_values_in(
            &mut self.0,
            alerts::METHOD,
            schema,
            None,
            values,
            lang,
            dry_run,
        )
    }

    fn reset(
        &mut self,
        schema: &str,
        key: &str,
        lang: Lang,
        dry_run: bool,
    ) -> Result<Value, ApiError> {
        gsettings::reset_values_in(
            &mut self.0,
            alerts::METHOD,
            schema,
            None,
            &[key],
            lang,
            dry_run,
        )
    }
}

fn mem() -> MemKeys {
    MemKeys(gsettings::Env::new(
        &gsettings::Config {
            backend: gsettings::Backend::Memory,
            ..Default::default()
        },
        None,
    ))
}

fn temp_dirs(tag: &str) -> alerts::Dirs {
    let root = std::env::temp_dir().join(format!("sound-alerts-{tag}-{}", crate::events::nonce()));
    let alerts_dir = root.join("alerts");
    std::fs::create_dir_all(&alerts_dir).unwrap();
    for s in alerts::SOUNDS {
        std::fs::write(alerts_dir.join(format!("{s}.ogg")), b"OggS").unwrap();
    }
    alerts::Dirs {
        data: root.join("data"),
        alerts: alerts_dir,
    }
}

#[test]
fn the_alert_chooser_value_follows_gnome_settings() {
    assert_eq!(alerts::alert_of(false, "__custom", Some("click")), "none");
    assert_eq!(alerts::alert_of(true, "Yaru", Some("click")), "default");
    assert_eq!(alerts::alert_of(true, "__custom", Some("hum")), "hum");
    assert_eq!(
        alerts::alert_of(true, "__custom", Some("bark")),
        "default",
        "an old GNOME sound"
    );
    assert_eq!(alerts::alert_of(true, "__custom", None), "default");
    let lang = Lang::En;
    let mut keys = mem();
    let plan = |args: Value, keys: &mut MemKeys| alerts::plan(&args, keys, lang);
    assert_eq!(
        plan(json!({"alert": "none"}), &mut keys).unwrap().sound,
        vec![("event-sounds", json!(false))]
    );
    let p = plan(json!({"alert": "swing", "startupSound": true}), &mut keys).unwrap();
    assert_eq!(p.custom, Some("swing"));
    assert_eq!(
        p.sound,
        vec![
            ("event-sounds", json!(true)),
            ("theme-name", json!("__custom"))
        ]
    );
    assert_eq!(
        p.startup,
        Some(json!("warty-startup")),
        "GNOME Settings' startup-sound mapping on Ubuntu"
    );
    let p = plan(json!({"alert": "default"}), &mut keys).unwrap();
    assert_eq!(p.sound, vec![("event-sounds", json!(true))]);
    assert_eq!(
        p.reset,
        vec!["theme-name"],
        "Standaard resets theme-name, as GNOME Settings does"
    );
    assert_eq!(
        plan(json!({"startupSound": false}), &mut keys)
            .unwrap()
            .startup,
        Some(json!(""))
    );
    for bad in [
        json!({}),
        json!({"alert": "bark"}),
        json!({"theme": "../x"}),
        json!({"theme": "a/b"}),
        json!({"alert": "click", "theme": "Yaru"}),
        json!({"alert": "default", "theme": "Yaru"}),
        json!({"eventSounds": "yes"}),
        json!({"startupSound": 1}),
    ] {
        assert!(plan(bad.clone(), &mut keys).is_err(), "{bad}");
    }
}

#[test]
fn a_custom_alert_writes_gnome_settings_theme_and_keys() {
    let dirs = temp_dirs("custom");
    let mut keys = mem();
    let lang = Lang::Nl;
    let out = alerts::set(&dirs, &mut keys, &json!({"alert": "click"}), lang, false).unwrap();
    assert_eq!(out["alert"], "click");
    assert_eq!(out["eventSounds"], true);
    assert_eq!(out["theme"], "__custom");
    let theme = dirs.theme();
    let index = std::fs::read_to_string(theme.join("index.theme")).unwrap();
    assert!(
        index.contains("[Sound Theme]")
            && index.contains("Name=Custom")
            && index.contains("Directories=."),
        "{index}"
    );
    let inherits = alerts::Keys::read(&mut keys, alerts::SCHEMA, "theme-name", lang)["default"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(index.contains(&format!("Inherits={inherits}")), "{index}");
    for link in ["bell-terminal.ogg", "bell-window-system.ogg"] {
        assert_eq!(
            std::fs::read_link(theme.join(link)).unwrap(),
            dirs.sound("click")
        );
    }
    // another sound relinks; the index keeps what else it held
    std::fs::write(
        theme.join("index.theme"),
        format!("{index}\n[Extra]\nKeep=1\n"),
    )
    .unwrap();
    let out = alerts::set(
        &dirs,
        &mut keys,
        &json!({"alert": "hum", "startupSound": true}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(out["alert"], "hum");
    assert_eq!(
        std::fs::read_link(theme.join("bell-window-system.ogg")).unwrap(),
        dirs.sound("hum")
    );
    assert!(
        std::fs::read_to_string(theme.join("index.theme"))
            .unwrap()
            .contains("Keep=1")
    );
    assert_eq!(out["startupSound"], true);
    assert_eq!(
        alerts::Keys::read(&mut keys, alerts::UBUNTU, "startup-sound", lang)["value"],
        "warty-startup"
    );
    // Geen geluiden, then Standaard
    assert_eq!(
        alerts::set(&dirs, &mut keys, &json!({"alert": "none"}), lang, false).unwrap()["alert"],
        "none"
    );
    let out = alerts::set(&dirs, &mut keys, &json!({"alert": "default"}), lang, false).unwrap();
    assert_eq!(out["alert"], "default");
    assert_eq!(out["theme"], json!(inherits));
    let theme_name = alerts::Keys::read(&mut keys, alerts::SCHEMA, "theme-name", lang);
    assert_eq!(
        theme_name["isDefault"], true,
        "Standaard leaves theme-name at its default, not a copy of it: {theme_name}"
    );
    let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
}

#[test]
fn standaard_resets_the_theme_and_a_dry_run_keeps_it() {
    let dirs = temp_dirs("reset");
    let mut keys = mem();
    let lang = Lang::En;
    alerts::set(&dirs, &mut keys, &json!({"alert": "swing"}), lang, false).unwrap();
    let read = |keys: &mut MemKeys| alerts::Keys::read(keys, alerts::SCHEMA, "theme-name", lang);
    assert_eq!(read(&mut keys)["value"], alerts::CUSTOM);
    let out = alerts::set(&dirs, &mut keys, &json!({"alert": "default"}), lang, true).unwrap();
    assert_eq!(out["dryRun"], true);
    assert_eq!(
        read(&mut keys)["value"],
        alerts::CUSTOM,
        "a dry run resets nothing"
    );
    assert_eq!(read(&mut keys)["isDefault"], false);
    let out = alerts::set(&dirs, &mut keys, &json!({"alert": "default"}), lang, false).unwrap();
    assert_eq!(out["alert"], "default");
    assert_eq!(out["eventSounds"], true);
    let item = read(&mut keys);
    assert_eq!(item["isDefault"], true, "{item}");
    assert_eq!(out["theme"], item["default"]);
    assert_eq!(out["defaultTheme"], item["default"]);
    let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
}

#[test]
fn an_alert_dry_run_or_refusal_writes_nothing() {
    let dirs = temp_dirs("dry");
    let mut keys = mem();
    let lang = Lang::En;
    let before = alerts::get(&dirs, &mut keys, lang);
    let out = alerts::set(
        &dirs,
        &mut keys,
        &json!({"alert": "string", "startupSound": true}),
        lang,
        true,
    )
    .unwrap();
    assert_eq!(out["dryRun"], true);
    assert!(!dirs.theme().exists(), "no theme file in a dry run");
    assert_eq!(
        alerts::get(&dirs, &mut keys, lang),
        before,
        "no key in a dry run"
    );
    // a missing alert file is refused before anything is written
    std::fs::remove_file(dirs.sound("swing")).unwrap();
    let before = alerts::get(&dirs, &mut keys, lang);
    let err = alerts::set(&dirs, &mut keys, &json!({"alert": "swing"}), lang, false).unwrap_err();
    assert_eq!(err.key.as_deref(), Some("sound.alertMissing"));
    assert!(!dirs.theme().exists());
    assert_eq!(alerts::get(&dirs, &mut keys, lang), before);
    let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
}

#[test]
fn the_sound_keys_have_their_allow_rows() {
    let mut keys = mem();
    let lang = Lang::En;
    for (schema, key, mode) in [
        (alerts::SCHEMA, "event-sounds", "service"),
        (alerts::SCHEMA, "theme-name", "service"),
        (alerts::SCHEMA, "allow-volume-above-100-percent", "write"),
        (alerts::UBUNTU, "startup-sound", "write"),
    ] {
        let item = alerts::Keys::read(&mut keys, schema, key, lang);
        assert_eq!(item["allowed"], true, "{schema} {key}");
        assert_eq!(item["mode"], mode, "{schema} {key}");
    }
    assert_eq!(
        alerts::Keys::read(&mut keys, alerts::SCHEMA, "theme-name", lang)["method"],
        alerts::METHOD
    );
    // another method may not write sound.alerts.set's rows, and gsettings.set may not either
    let mut env = gsettings::Env::new(
        &gsettings::Config {
            backend: gsettings::Backend::Memory,
            ..Default::default()
        },
        None,
    );
    let err = gsettings::write_values_in(
        &mut env,
        "sound.setVolume",
        alerts::SCHEMA,
        None,
        &[("theme-name", json!("x"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.useMethod"));
}
