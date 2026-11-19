//! What the Sound service knows of the PulseAudio server (pipewire-pulse), as plain Rust data: the client fills it
//! from libpulse's introspection and subscription (client.rs), `state_json` gives the pages `sound.state`, and
//! `devices` is GNOME Settings' list of output and input devices (libgvc's "UI devices": one entry per available
//! card port, a profile switch when the port's profile is not the active one, one entry for a sink or source without
//! a card). Pure, so the tests build it from fixtures.
use super::pulse::{self, CVolume, ChannelMap, Pulse};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) type Props = BTreeMap<String, String>;

/// A port of a sink or source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Port {
    pub name: String,
    pub description: String,
    pub priority: u32,
    /// "yes", "no" or "unknown" (pa_port_available_t)
    pub available: &'static str,
}

/// A sink or a source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Device {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub card: Option<u32>,
    pub ports: Vec<Port>,
    pub active_port: Option<String>,
    pub volume: Vec<u32>,
    pub map: Vec<i32>,
    pub mute: bool,
    pub base_volume: u32,
    pub decibel: bool,
    pub hw_volume: bool,
    /// a sink's monitor source name
    pub monitor_name: Option<String>,
    /// a monitor source's sink
    pub monitor_of: Option<u32>,
    pub state: i32,
    pub props: Props,
}

/// A sink input (an app's playback) or a source output (an app's recording).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stream {
    pub index: u32,
    pub name: String,
    pub client: Option<u32>,
    pub device: u32,
    pub volume: Vec<u32>,
    pub map: Vec<i32>,
    pub mute: bool,
    pub corked: bool,
    pub has_volume: bool,
    pub volume_writable: bool,
    pub props: Props,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Client {
    pub index: u32,
    pub name: String,
    pub props: Props,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Profile {
    pub name: String,
    pub description: String,
    pub sinks: u32,
    pub sources: u32,
    pub priority: u32,
    pub available: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CardPort {
    pub name: String,
    pub description: String,
    pub priority: u32,
    pub available: &'static str,
    /// true: an output port (pa_direction_t OUTPUT), false: input
    pub output: bool,
    pub profiles: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Card {
    pub index: u32,
    pub name: String,
    pub driver: String,
    pub profiles: Vec<Profile>,
    pub active_profile: Option<String>,
    pub ports: Vec<CardPort>,
    pub props: Props,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Server {
    pub name: String,
    pub version: String,
    pub default_sink: String,
    pub default_source: String,
}

/// module-stream-restore's "sink-input-by-media-role:event" (GNOME's "System Sounds").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EventRole {
    pub volume: Vec<u32>,
    pub map: Vec<i32>,
    pub mute: bool,
    pub device: Option<String>,
    /// false: no entry yet (GNOME shows 100 % until one is written)
    pub stored: bool,
}

impl Default for EventRole {
    fn default() -> Self {
        EventRole {
            volume: vec![pulse::VOLUME_NORM],
            map: vec![pulse::POSITION_MONO],
            mute: false,
            device: None,
            stored: false,
        }
    }
}

pub(crate) const EVENT_ROLE: &str = "sink-input-by-media-role:event";

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Model {
    pub server: Server,
    pub sinks: BTreeMap<u32, Device>,
    pub sources: BTreeMap<u32, Device>,
    pub sink_inputs: BTreeMap<u32, Stream>,
    pub source_outputs: BTreeMap<u32, Stream>,
    pub clients: BTreeMap<u32, Client>,
    pub cards: BTreeMap<u32, Card>,
    pub event: EventRole,
    /// the profile a card had before Settings turned it off (Audio: Niet toestaan), for Toestaan
    pub previous_profiles: BTreeMap<String, String>,
}

/// Percent of PA_VOLUME_NORM, as GNOME's sliders and wpctl show it.
pub(crate) fn pct(v: u32) -> f64 {
    v as f64 * 100.0 / pulse::VOLUME_NORM as f64
}

/// A percentage as pa_volume_t.
pub(crate) fn from_pct(p: f64) -> u32 {
    (p.max(0.0) * pulse::VOLUME_NORM as f64 / 100.0)
        .round()
        .min(u32::MAX as f64 / 2.0) as u32
}

/// GNOME's volume of a cvolume: its loudest channel (gvc_mixer_stream: pa_cvolume_max).
pub(crate) fn max_of(values: &[u32]) -> u32 {
    values.iter().copied().max().unwrap_or(0)
}

/// The percentage the UI shows (whole percent).
pub(crate) fn ui_pct(values: &[u32]) -> i64 {
    pct(max_of(values)).round() as i64
}

/// Sliders for a left/right pair (Windows' Linkerkanaal / Rechterkanaal): from GNOME's balance and volume, the left
/// and right volume in percent (pa_cvolume_set_balance's inverse: the louder side has the volume).
pub(crate) fn left_right(volume: f64, balance: f64) -> (f64, f64) {
    let b = balance.clamp(-1.0, 1.0);
    if b <= 0.0 {
        (volume, volume * (1.0 + b))
    } else {
        (volume * (1.0 - b), volume)
    }
}

/// The volume and balance that give `left` and `right` (pa_cvolume_get_balance's own formula).
pub(crate) fn volume_balance(left: f64, right: f64) -> (f64, f64) {
    let (l, r) = (left.max(0.0), right.max(0.0));
    let volume = l.max(r);
    let balance = if (l - r).abs() < 1e-9 || volume <= 0.0 {
        0.0
    } else if l > r {
        -1.0 + r / l
    } else {
        1.0 - l / r
    };
    (volume, balance)
}

fn position_name(lib: Option<&Pulse>, p: i32) -> String {
    match lib {
        Some(lib) => pulse::text(unsafe { (lib.pa_channel_position_to_string)(p) }),
        None => format!("{p}"),
    }
}

/// Balance, fade and LFE of a device's volume through libpulse (GNOME's sliders), when it can say.
fn balance_of(lib: Option<&Pulse>, volume: &[u32], map: &[i32]) -> Value {
    let (Some(lib), true) = (lib, volume.len() == map.len() && !map.is_empty()) else {
        return json!({"canBalance": false, "canFade": false, "hasLfe": false, "balance": 0.0, "fade": 0.0, "lfe": null});
    };
    let cv = CVolume::of(volume);
    let cm = ChannelMap::of(map);
    unsafe {
        let can_balance = (lib.pa_channel_map_can_balance)(&cm) != 0;
        let can_fade = (lib.pa_channel_map_can_fade)(&cm) != 0;
        let has_lfe = (lib.pa_channel_map_has_position)(&cm, pulse::POSITION_LFE) != 0;
        let balance = if can_balance {
            (lib.pa_cvolume_get_balance)(&cv, &cm) as f64
        } else {
            0.0
        };
        let fade = if can_fade {
            (lib.pa_cvolume_get_fade)(&cv, &cm) as f64
        } else {
            0.0
        };
        let lfe = has_lfe
            .then(|| pct((lib.pa_cvolume_get_position)(&cv, &cm, pulse::POSITION_LFE)).round());
        json!({"canBalance": can_balance, "canFade": can_fade, "hasLfe": has_lfe,
            "balance": (balance * 1000.0).round() / 1000.0, "fade": (fade * 1000.0).round() / 1000.0, "lfe": lfe})
    }
}

fn state_name(state: i32) -> &'static str {
    match state {
        0 => "running",
        1 => "idle",
        2 => "suspended",
        _ => "unknown",
    }
}

/// A stream's application id is one of the level meters GNOME hides (GNOME Settings' and pavucontrol's, and so
/// WinPoint's own, which carry GNOME Settings' id: client.rs `METER_APP_ID`).
pub(crate) fn is_meter(props: &Props) -> bool {
    matches!(
        props.get("application.id").map(String::as_str),
        Some("org.gnome.VolumeControl" | "org.PulseAudio.pavucontrol")
    )
}

impl Model {
    pub(crate) fn card(&self, index: Option<u32>) -> Option<&Card> {
        index.and_then(|i| self.cards.get(&i))
    }

    pub(crate) fn card_named(&self, name: &str) -> Option<&Card> {
        self.cards.values().find(|c| c.name == name)
    }

    pub(crate) fn sink_named(&self, name: &str) -> Option<&Device> {
        self.sinks.values().find(|d| d.name == name)
    }

    pub(crate) fn source_named(&self, name: &str) -> Option<&Device> {
        self.sources.values().find(|d| d.name == name)
    }

    pub(crate) fn default_sink(&self) -> Option<&Device> {
        self.sink_named(&self.server.default_sink)
    }

    fn device_json(&self, lib: Option<&Pulse>, d: &Device, sink: bool) -> Value {
        let card = self.card(d.card);
        let channels: Vec<Value> = d
            .map
            .iter()
            .zip(d.volume.iter())
            .map(|(p, v)| json!({"position": position_name(lib, *p), "volume": pct(*v).round()}))
            .collect();
        let mut v = json!({
            "index": d.index,
            "name": d.name,
            "description": d.description,
            "card": card.map(|c| c.name.clone()),
            "cardIndex": d.card,
            "ports": d.ports.iter().map(|p| json!({"name": p.name, "description": p.description, "available": p.available, "priority": p.priority})).collect::<Vec<_>>(),
            "activePort": d.active_port,
            "volume": ui_pct(&d.volume),
            "channels": channels,
            "mute": d.mute,
            "default": if sink { d.name == self.server.default_sink } else { d.name == self.server.default_source },
            "baseVolume": pct(d.base_volume).round(),
            "decibel": d.decibel,
            "state": state_name(d.state),
            "icon": d.props.get("device.icon_name"),
            "formFactor": d.props.get("device.form_factor"),
            "bus": d.props.get("device.bus"),
            "objectId": d.props.get("object.id").and_then(|s| s.parse::<i64>().ok()),
        });
        if sink {
            v["monitor"] = json!(d.monitor_name);
        } else {
            v["monitorOf"] = json!(
                d.monitor_of
                    .and_then(|i| self.sinks.get(&i))
                    .map(|s| s.name.clone())
            );
        }
        let balance = balance_of(lib, &d.volume, &d.map);
        if let (Some(o), Some(b)) = (v.as_object_mut(), balance.as_object()) {
            for (k, x) in b {
                o.insert(k.clone(), x.clone());
            }
        }
        // Windows' Linkerkanaal / Rechterkanaal
        let (left, right) = left_right(
            pct(max_of(&d.volume)),
            balance["balance"].as_f64().unwrap_or(0.0),
        );
        v["left"] = json!(left.round());
        v["right"] = json!(right.round());
        v
    }

    /// The app a stream belongs to: its application name (the client's as fallback), icon and binary.
    fn app_of(&self, s: &Stream) -> (String, Option<String>) {
        let client = s.client.and_then(|c| self.clients.get(&c));
        let pick = |key: &str| {
            s.props
                .get(key)
                .cloned()
                .or_else(|| client.and_then(|c| c.props.get(key).cloned()))
                .filter(|x| !x.is_empty())
        };
        let name = pick("application.name")
            .or_else(|| client.map(|c| c.name.clone()).filter(|x| !x.is_empty()))
            .unwrap_or_else(|| s.name.clone());
        let icon = pick("application.icon_name")
            .or_else(|| pick("media.icon_name"))
            .or_else(|| pick("window.icon_name"));
        (name, icon)
    }

    fn stream_json(&self, s: &Stream, output: bool) -> Value {
        let (app, icon) = self.app_of(s);
        let device = if output {
            self.sinks.get(&s.device)
        } else {
            self.sources.get(&s.device)
        };
        json!({
            "index": s.index,
            "kind": if output { "output" } else { "input" },
            "app": app,
            "icon": icon,
            "appId": s.props.get("application.id"),
            "name": s.props.get("media.name").cloned().unwrap_or_else(|| s.name.clone()),
            "role": s.props.get("media.role"),
            "volume": ui_pct(&s.volume),
            "mute": s.mute,
            "corked": s.corked,
            "hasVolume": s.has_volume,
            "volumeWritable": s.volume_writable,
            "device": device.map(|d| d.name.clone()),
        })
    }

    /// The streams GNOME's Volume Levels lists: every app's playback and recording, without the event-role streams
    /// ("Systeemgeluiden" is one slider) and without the level meters of GNOME Settings, pavucontrol and WinPoint,
    /// and (for recordings) without the ones that record a monitor.
    pub(crate) fn listed_streams(&self) -> Vec<(bool, &Stream)> {
        let mut out = Vec::new();
        for s in self.sink_inputs.values() {
            if is_meter(&s.props) || s.props.get("media.role").map(String::as_str) == Some("event")
            {
                continue;
            }
            out.push((true, s));
        }
        for s in self.source_outputs.values() {
            if is_meter(&s.props)
                || self
                    .sources
                    .get(&s.device)
                    .is_some_and(|d| d.monitor_of.is_some())
            {
                continue;
            }
            out.push((false, s));
        }
        out
    }

    fn card_json(&self, c: &Card) -> Value {
        let off = c.active_profile.as_deref() == Some("off");
        let restore = self
            .previous_profiles
            .get(&c.name)
            .filter(|p| c.profiles.iter().any(|x| &x.name == *p))
            .cloned()
            .or_else(|| best_profile(c, None).map(str::to_owned));
        json!({
            "index": c.index,
            "name": c.name,
            "description": card_description(c),
            "driver": c.driver,
            "profiles": c.profiles.iter().map(|p| json!({"name": p.name, "description": p.description, "available": p.available, "priority": p.priority, "sinks": p.sinks, "sources": p.sources})).collect::<Vec<_>>(),
            "activeProfile": c.active_profile,
            "off": off,
            "restore": restore,
            "ports": c.ports.iter().map(|p| json!({"name": p.name, "description": p.description, "direction": if p.output { "output" } else { "input" }, "available": p.available, "profiles": p.profiles})).collect::<Vec<_>>(),
            "outputs": c.ports.iter().any(|p| p.output) || c.profiles.iter().any(|p| p.sinks > 0),
            "inputs": c.ports.iter().any(|p| !p.output) || c.profiles.iter().any(|p| p.sources > 0),
            "icon": c.props.get("device.icon_name"),
            "formFactor": c.props.get("device.form_factor"),
            "bus": c.props.get("device.bus"),
        })
    }

    /// sound.state (§5.5): everything the four sound pages show. `max_volume` is GNOME's amplified maximum in
    /// percent (pa_sw_volume_from_dB(11.0), about 153 %).
    pub(crate) fn state_json(&self, lib: Option<&Pulse>, max_volume: f64) -> Value {
        let sinks: Vec<Value> = self
            .sinks
            .values()
            .map(|d| self.device_json(lib, d, true))
            .collect();
        let sources: Vec<Value> = self
            .sources
            .values()
            .map(|d| self.device_json(lib, d, false))
            .collect();
        let streams: Vec<Value> = self
            .listed_streams()
            .into_iter()
            .map(|(out, s)| self.stream_json(s, out))
            .collect();
        json!({
            "available": true,
            "server": self.server.name,
            "version": self.server.version,
            "defaultSink": self.server.default_sink,
            "defaultSource": self.server.default_source,
            "maxVolume": max_volume.round(),
            "outputs": devices(self, true),
            "inputs": devices(self, false),
            "sinks": sinks,
            "sources": sources,
            "cards": self.cards.values().map(|c| self.card_json(c)).collect::<Vec<_>>(),
            "streams": streams,
            "eventVolume": ui_pct(&self.event.volume),
            "eventMute": self.event.mute,
        })
    }
}

/// A card's name for people: its device.description (pipewire-pulse fills it), else the card name.
pub(crate) fn card_description(c: &Card) -> String {
    c.props
        .get("device.description")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| c.name.clone())
}

/// The profile GNOME switches to for a port that is not in the active profile (gvc_mixer_ui_device_get_best_profile):
/// among the port's available profiles, one that keeps the other direction of the active profile ("output:hdmi-stereo
/// +input:analog-stereo" when the input was analog), then the highest priority. `port` None: the card's best
/// available profile other than "off" (Toestaan without a remembered profile).
pub(crate) fn best_profile<'a>(card: &'a Card, port: Option<&CardPort>) -> Option<&'a str> {
    let keep: Vec<&str> = match (port, card.active_profile.as_deref()) {
        (Some(p), Some(active)) => active
            .split('+')
            .filter(|part| {
                if p.output {
                    part.starts_with("input:")
                } else {
                    part.starts_with("output:")
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    let candidates: Vec<&Profile> = card
        .profiles
        .iter()
        .filter(|pr| pr.available && pr.name != "off")
        .filter(|pr| port.is_none_or(|p| p.profiles.iter().any(|n| n == &pr.name)))
        .collect();
    candidates
        .iter()
        .max_by_key(|pr| {
            (
                keep.iter()
                    .filter(|k| pr.name.split('+').any(|x| x == **k))
                    .count(),
                pr.priority,
            )
        })
        .map(|pr| pr.name.as_str())
}

/// GNOME Settings' device list of one direction (libgvc's UI devices; Windows' "Kies waar geluid moet worden
/// afgespeeld" and "Kies een apparaat om te spreken of op te nemen"): every available port of every card (a port
/// whose profile is not active carries the profile to switch to), then every sink or source without a card or
/// without ports; monitors are never inputs.
pub(crate) fn devices(m: &Model, output: bool) -> Vec<Value> {
    let list = if output { &m.sinks } else { &m.sources };
    let default = if output {
        &m.server.default_sink
    } else {
        &m.server.default_source
    };
    let mut out = Vec::new();
    let mut covered: Vec<u32> = Vec::new();
    for card in m.cards.values() {
        let ports: Vec<&CardPort> = card
            .ports
            .iter()
            .filter(|p| p.output == output && p.available != "no")
            .collect();
        for port in ports {
            let dev = list.values().find(|d| {
                d.card == Some(card.index) && d.ports.iter().any(|p| p.name == port.name)
            });
            let profile = match dev {
                Some(_) => None,
                None => match best_profile(card, Some(port)) {
                    Some(p) => Some(p.to_owned()),
                    None => continue,
                },
            };
            if let Some(d) = dev {
                covered.push(d.index);
            }
            let is_default = dev.is_some_and(|d| {
                &d.name == default
                    && (d.ports.len() < 2 || d.active_port.as_deref() == Some(port.name.as_str()))
            });
            // Windows: the endpoint ("Luidsprekers") with the adapter below it; GNOME: "Speakers – Built-in Audio"
            let title = if port.description.is_empty() {
                dev.map(|d| d.description.clone())
                    .unwrap_or_else(|| port.name.clone())
            } else {
                port.description.clone()
            };
            out.push(json!({
                "id": format!("{}:{}", card.name, port.name),
                "kind": if output { "sink" } else { "source" },
                "title": title,
                "subtitle": card_description(card),
                "device": dev.map(|d| d.name.clone()),
                "port": port.name,
                "card": card.name,
                "profile": profile,
                "default": is_default,
                "available": port.available,
                "icon": dev.and_then(|d| d.props.get("device.icon_name").cloned()).or_else(|| card.props.get("device.icon_name").cloned()),
                "formFactor": card.props.get("device.form_factor"),
                "bus": card.props.get("device.bus"),
            }));
        }
    }
    for d in list.values() {
        if covered.contains(&d.index) || d.monitor_of.is_some() {
            continue;
        }
        let card = m.card(d.card);
        out.push(json!({
            "id": d.name,
            "kind": if output { "sink" } else { "source" },
            "title": d.description,
            "subtitle": card.map(card_description).unwrap_or_default(),
            "device": d.name,
            "port": d.active_port,
            "card": card.map(|c| c.name.clone()),
            "profile": null,
            "default": &d.name == default,
            "available": "unknown",
            "icon": d.props.get("device.icon_name"),
            "formFactor": d.props.get("device.form_factor"),
            "bus": d.props.get("device.bus"),
        }));
    }
    out
}
