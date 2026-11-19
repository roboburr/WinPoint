//! `sound.*` (batch D, notes/claude/settings-parity.md §5.5, amendment 39): GNOME Settings' Sound panel on the
//! PulseAudio protocol that pipewire-pulse speaks, as GNOME's libgvc does, through libpulse on the process's own
//! sound thread (sound/client.rs), for Windows' Geluid, Eigenschappen, Alle geluidsapparaten and Volumemixer.
//!
//! * `sound.state` → the devices (GNOME's output and input lists: card ports, profile switches), sinks, sources,
//!   cards with their profiles and ports, the apps' streams and the event role (Systeemgeluiden); event
//!   `sound.changed` with the same state after every change (one per burst), `sound.level {kind, name, peak}` from
//!   a running meter.
//! * Writes: setDefault, setVolume, setMute, setBalance (balance, fade, subwoofer), setProfile (also "off": Audio
//!   Niet toestaan), setPort, moveStream, setEventVolume, `sound.alerts.set` (sound/alerts.rs). A dry run checks the
//!   arguments against the server's state and writes nothing.
//! * `sound.test` plays GNOME's speaker test or an alert (sound/canberra.rs), `sound.meter` starts a level meter.
//! * system.audio.* (system/audio.rs, the taskbar and Snelle instellingen) delegates here while the client is
//!   connected ([`audio_state`] and friends); wpctl stays its fallback.
pub(crate) mod alerts;
mod canberra;
pub(crate) mod client;
pub(crate) mod model;
pub(crate) mod pulse;
#[cfg(test)]
mod tests;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use client::{Client, Event, Fail, Reply};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The methods (name, dangerous), exactly as host/src/api-methods/system.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("sound.state", false),
    ("sound.setDefault", true),
    ("sound.setVolume", true),
    ("sound.setMute", true),
    ("sound.setBalance", true),
    ("sound.setProfile", true),
    ("sound.setPort", true),
    ("sound.moveStream", true),
    ("sound.setEventVolume", true),
    ("sound.test", false),
    ("sound.meter", false),
    ("sound.alerts.get", false),
    ("sound.alerts.set", true),
];

/// How long a call waits for the first connection to the server.
const FIRST_CONNECT: Duration = Duration::from_millis(2000);
/// How long setDefault waits for the device a profile switch brings.
const PROFILE_WAIT: Duration = Duration::from_secs(3);

const SOUND_SCHEMA: &str = "org.gnome.desktop.sound";
const AMPLIFY: &str = "allow-volume-above-100-percent";

pub(crate) struct Sound {
    ctx: Arc<ServiceContext>,
    client: Option<Arc<Client>>,
    listener: Mutex<Option<u64>>,
}

/// sound.state without a server.
pub(crate) fn unavailable_state() -> Value {
    json!({"available": false, "outputs": [], "inputs": [], "sinks": [], "sources": [], "cards": [], "streams": [],
        "eventVolume": null, "eventMute": false, "maxVolume": 100})
}

fn state_of(c: &Client) -> Value {
    match c.snapshot() {
        Some(m) => m.state_json(pulse::libs().map(|l| &l.pa), c.max_volume_pct()),
        None => unavailable_state(),
    }
}

/// An error in the caller's language (stem `sound`).
pub(crate) fn fail(f: Fail, lang: Lang) -> ApiError {
    match f {
        Fail::Unavailable => ApiError::tr("unsupported", lang, "sound.unavailable", NO_PARAMS),
        Fail::Gone => ApiError::tr("not_found", lang, "sound.deviceGone", NO_PARAMS),
        Fail::Bad => ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS),
        Fail::Failed(message) => {
            let mut e = ApiError::tr("failed", lang, "sound.failed", NO_PARAMS);
            if !message.is_empty() {
                e.data = Some(json!({ "error": message }));
            }
            e
        }
    }
}

fn bad(lang: Lang) -> ApiError {
    fail(Fail::Bad, lang)
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 512)
}

fn index_arg(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n != pulse::INVALID_INDEX)
}

/// What a volume, mute or move call addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Sink,
    Source,
    SinkInput,
    SourceOutput,
}

pub(crate) fn target(args: &Value) -> Option<Target> {
    match args.get("kind").and_then(Value::as_str)? {
        "sink" | "output" => Some(Target::Sink),
        "source" | "input" => Some(Target::Source),
        "sinkInput" | "playback" => Some(Target::SinkInput),
        "sourceOutput" | "recording" => Some(Target::SourceOutput),
        _ => None,
    }
}

/// A device kind: "sink" (true) or "source" (false).
fn device_kind(args: &Value, lang: Lang) -> Result<bool, ApiError> {
    match target(args) {
        Some(Target::Sink) => Ok(true),
        Some(Target::Source) => Ok(false),
        _ => Err(bad(lang)),
    }
}

/// A volume in percent, 0 up to `max`.
pub(crate) fn volume_arg(v: &Value, max: f64) -> Option<f64> {
    let n = v.as_f64().filter(|n| n.is_finite())?;
    (0.0..=1000.0).contains(&n).then(|| n.min(max))
}

/// The volume limit: 100 %, or GNOME's amplified maximum while allow-volume-above-100-percent is on.
fn max_volume(c: &Client, lang: Lang) -> f64 {
    let item = crate::services::gsettings::read_item(SOUND_SCHEMA, None, AMPLIFY, lang)
        .unwrap_or(Value::Null);
    if item["value"] == true {
        c.max_volume_pct()
    } else {
        100.0
    }
}

impl Sound {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let client = client::shared();
        let s = Arc::new(Self {
            ctx,
            client,
            listener: Mutex::new(None),
        });
        if let Some(c) = &s.client {
            let (ctx, weak) = (Arc::downgrade(&s.ctx), Arc::downgrade(c));
            let id = c.listen(Arc::new(move |event| {
                let Some(ctx) = ctx.upgrade() else { return };
                match event {
                    Event::Changed => {
                        let state = weak
                            .upgrade()
                            .map(|c| state_of(&c))
                            .unwrap_or_else(unavailable_state);
                        ctx.emit("sound.changed", state);
                    }
                    Event::Level { kind, name, peak } => ctx.emit(
                        "sound.level",
                        json!({"kind": kind, "name": name, "peak": peak}),
                    ),
                }
            }));
            *s.listener.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
        }
        Ok(s)
    }

    pub(crate) fn shutdown(&self) {
        if let (Some(c), Some(id)) = (
            &self.client,
            self.listener
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take(),
        ) {
            c.unlisten(id);
            let _ = c.run(|e| e.stop_meters());
        }
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        let lang = self.ctx.lang();
        Some(match method {
            "sound.alerts.get" => Ok(alerts::get(
                &alerts::Dirs::system(),
                &mut alerts::SystemKeys,
                lang,
            )),
            "sound.alerts.set" => alerts::set(
                &alerts::Dirs::system(),
                &mut alerts::SystemKeys,
                args,
                lang,
                self.ctx.dry_run,
            ),
            "sound.state" => Ok(self.state()),
            _ => self
                .client(lang)
                .and_then(|c| self.pulse_call(&c, method, args, lang)),
        })
    }

    fn state(&self) -> Value {
        match &self.client {
            Some(c) if c.wait_ready(FIRST_CONNECT) => state_of(c),
            _ => unavailable_state(),
        }
    }

    fn client(&self, lang: Lang) -> Result<Arc<Client>, ApiError> {
        match &self.client {
            Some(c) if c.wait_ready(FIRST_CONNECT) => Ok(c.clone()),
            _ => Err(fail(Fail::Unavailable, lang)),
        }
    }

    fn pulse_call(&self, c: &Arc<Client>, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let dry = self.ctx.dry_run;
        let done = |r: Result<(), Fail>| r.map(|_| json!({"ok": true})).map_err(|f| fail(f, lang));
        let dry_ok = || Ok(json!({"ok": true, "dryRun": true}));
        match method {
            "sound.setDefault" => self.set_default(c, args, lang),
            "sound.setVolume" => {
                let t = target(args).ok_or_else(|| bad(lang))?;
                let volume =
                    volume_arg(&args["volume"], max_volume(c, lang)).ok_or_else(|| bad(lang))?;
                let (name, index) = (
                    text_arg(args, "name").map(str::to_owned),
                    index_arg(args, "index"),
                );
                let job = move |e: &client::Engine, write: bool| -> Result<Vec<Reply>, Fail> {
                    let v = model::from_pct(volume);
                    match t {
                        Target::Sink | Target::Source => {
                            let sink = t == Target::Sink;
                            let d = e.device(sink, name.as_deref(), index)?;
                            if !write {
                                return Ok(vec![]);
                            }
                            let mut out =
                                vec![e.set_device_volume(&d, sink, e.scaled(&d.volume, v))];
                            if d.mute && v > 0 {
                                out.push(e.set_device_mute(&d, sink, false));
                            }
                            Ok(out)
                        }
                        Target::SinkInput | Target::SourceOutput => {
                            let output = t == Target::SinkInput;
                            let s = e.stream(output, index.ok_or(Fail::Bad)?)?;
                            if !s.has_volume || !s.volume_writable {
                                return Err(Fail::Bad);
                            }
                            if !write {
                                return Ok(vec![]);
                            }
                            let mut out =
                                vec![e.set_stream_volume(&s, output, e.scaled(&s.volume, v))];
                            if s.mute && v > 0 {
                                out.push(e.set_stream_mute(&s, output, false));
                            }
                            Ok(out)
                        }
                    }
                };
                if dry {
                    return done(c.write(move |e| job(e, false))).and_then(|_| dry_ok());
                }
                done(c.write(move |e| job(e, true)))
            }
            "sound.setMute" => {
                let t = target(args).ok_or_else(|| bad(lang))?;
                let mute = args["mute"].as_bool().ok_or_else(|| bad(lang))?;
                let (name, index) = (
                    text_arg(args, "name").map(str::to_owned),
                    index_arg(args, "index"),
                );
                let job = move |e: &client::Engine, write: bool| -> Result<Vec<Reply>, Fail> {
                    match t {
                        Target::Sink | Target::Source => {
                            let sink = t == Target::Sink;
                            let d = e.device(sink, name.as_deref(), index)?;
                            Ok(if write {
                                vec![e.set_device_mute(&d, sink, mute)]
                            } else {
                                vec![]
                            })
                        }
                        Target::SinkInput | Target::SourceOutput => {
                            let output = t == Target::SinkInput;
                            let s = e.stream(output, index.ok_or(Fail::Bad)?)?;
                            Ok(if write {
                                vec![e.set_stream_mute(&s, output, mute)]
                            } else {
                                vec![]
                            })
                        }
                    }
                };
                if dry {
                    return done(c.write(move |e| job(e, false))).and_then(|_| dry_ok());
                }
                done(c.write(move |e| job(e, true)))
            }
            "sound.setBalance" => {
                let sink = text_arg(args, "sink").ok_or_else(|| bad(lang))?.to_owned();
                let num = |k: &str, lo: f64, hi: f64| -> Result<Option<f64>, ApiError> {
                    match args.get(k).filter(|v| !v.is_null()) {
                        None => Ok(None),
                        Some(v) => v
                            .as_f64()
                            .filter(|n| n.is_finite() && (lo..=hi).contains(n))
                            .map(Some)
                            .ok_or_else(|| bad(lang)),
                    }
                };
                let max = max_volume(c, lang);
                let (mut balance, fade) = (num("balance", -1.0, 1.0)?, num("fade", -1.0, 1.0)?);
                let lfe = num("lfe", 0.0, 1000.0)?.map(|v| model::from_pct(v.min(max)));
                let mut volume = num("volume", 0.0, 1000.0)?.map(|v| model::from_pct(v.min(max)));
                // Windows' Linkerkanaal / Rechterkanaal: both sides in percent, as one volume and balance
                match (num("left", 0.0, 1000.0)?, num("right", 0.0, 1000.0)?) {
                    (Some(l), Some(r)) if balance.is_none() && volume.is_none() => {
                        let (v, b) = model::volume_balance(l.min(max), r.min(max));
                        volume = Some(model::from_pct(v));
                        balance = Some(b);
                    }
                    (None, None) => {}
                    _ => return Err(bad(lang)),
                }
                if balance.is_none() && fade.is_none() && lfe.is_none() && volume.is_none() {
                    return Err(bad(lang));
                }
                let write = !dry;
                let r = c.write(move |e| {
                    let d = e.device(true, Some(&sink), None)?;
                    let cv = e.balanced(&d, volume, balance, fade, lfe)?;
                    Ok(if write {
                        vec![e.set_device_volume(&d, true, cv)]
                    } else {
                        vec![]
                    })
                });
                if dry {
                    done(r).and_then(|_| dry_ok())
                } else {
                    done(r)
                }
            }
            "sound.setProfile" => {
                let card = text_arg(args, "card").ok_or_else(|| bad(lang))?.to_owned();
                let profile = text_arg(args, "profile")
                    .ok_or_else(|| bad(lang))?
                    .to_owned();
                if dry {
                    let snapshot = c.snapshot().ok_or_else(|| fail(Fail::Unavailable, lang))?;
                    let found = snapshot
                        .card_named(&card)
                        .ok_or_else(|| fail(Fail::Gone, lang))?;
                    if !found.profiles.iter().any(|p| p.name == profile) {
                        return Err(bad(lang));
                    }
                    return dry_ok();
                }
                done(c.write(move |e| Ok(vec![e.set_profile(&card, &profile)?])))
            }
            "sound.setPort" => {
                let sink = device_kind(args, lang)?;
                let name = text_arg(args, "name").ok_or_else(|| bad(lang))?.to_owned();
                let port = text_arg(args, "port").ok_or_else(|| bad(lang))?.to_owned();
                let write = !dry;
                let r = c.write(move |e| {
                    let d = e.device(sink, Some(&name), None)?;
                    if !d.ports.iter().any(|p| p.name == port) {
                        return Err(Fail::Bad);
                    }
                    Ok(if write {
                        vec![e.set_port(&d, sink, &port)?]
                    } else {
                        vec![]
                    })
                });
                if dry {
                    done(r).and_then(|_| dry_ok())
                } else {
                    done(r)
                }
            }
            "sound.moveStream" => {
                let output = match target(args) {
                    Some(Target::SinkInput) => true,
                    Some(Target::SourceOutput) => false,
                    _ => return Err(bad(lang)),
                };
                let index = index_arg(args, "index").ok_or_else(|| bad(lang))?;
                let device = text_arg(args, "device")
                    .ok_or_else(|| bad(lang))?
                    .to_owned();
                if dry {
                    let snapshot = c.snapshot().ok_or_else(|| fail(Fail::Unavailable, lang))?;
                    let streams = if output {
                        &snapshot.sink_inputs
                    } else {
                        &snapshot.source_outputs
                    };
                    let device_ok = if output {
                        snapshot.sink_named(&device).is_some()
                    } else {
                        snapshot.source_named(&device).is_some()
                    };
                    if !streams.contains_key(&index) || !device_ok {
                        return Err(fail(Fail::Gone, lang));
                    }
                    return dry_ok();
                }
                done(c.write(move |e| Ok(vec![e.move_stream(output, index, &device)?])))
            }
            "sound.setEventVolume" => {
                let volume = volume_arg(&args["volume"], 100.0).ok_or_else(|| bad(lang))?;
                let mute = match args.get("mute").filter(|v| !v.is_null()) {
                    None => None,
                    Some(m) => Some(m.as_bool().ok_or_else(|| bad(lang))?),
                };
                if dry {
                    return dry_ok();
                }
                done(c.write(move |e| Ok(vec![e.set_event_volume(model::from_pct(volume), mute)?])))
            }
            "sound.meter" => {
                let sink = device_kind(args, lang)?;
                let name = text_arg(args, "name").ok_or_else(|| bad(lang))?.to_owned();
                let on = args["on"].as_bool().ok_or_else(|| bad(lang))?;
                match c.run(move |e| e.meter(sink, &name, on)) {
                    Some(Ok(())) => {
                        Ok(json!({"ok": true, "on": on, "lease": client::METER_LEASE.as_secs()}))
                    }
                    Some(Err(f)) => Err(fail(f, lang)),
                    None => Err(fail(Fail::Unavailable, lang)),
                }
            }
            "sound.test" => self.test(c, args, lang),
            _ => Err(bad(lang)),
        }
    }

    /// sound.setDefault {kind, name?, port?, card?, profile?}: GNOME's gvc_mixer_control_change_output /
    /// _change_input: when the chosen port needs another card profile, switch it and wait for its device; then the
    /// port, then the server's default device.
    fn set_default(&self, c: &Arc<Client>, args: &Value, lang: Lang) -> ApiResult {
        let sink = device_kind(args, lang)?;
        let mut name = text_arg(args, "name").map(str::to_owned);
        let port = text_arg(args, "port").map(str::to_owned);
        let card = text_arg(args, "card").map(str::to_owned);
        let profile = text_arg(args, "profile").map(str::to_owned);
        let snapshot = c.snapshot().ok_or_else(|| fail(Fail::Unavailable, lang))?;
        let device_of = |m: &model::Model, index: u32| -> Option<String> {
            let list = if sink { &m.sinks } else { &m.sources };
            list.values()
                .find(|d| {
                    d.card == Some(index)
                        && d.monitor_of.is_none()
                        && port
                            .as_ref()
                            .is_none_or(|p| d.ports.iter().any(|x| &x.name == p))
                })
                .map(|d| d.name.clone())
        };
        if let (Some(card), Some(profile)) = (&card, &profile) {
            let found = snapshot
                .card_named(card)
                .ok_or_else(|| fail(Fail::Gone, lang))?;
            if !found.profiles.iter().any(|p| &p.name == profile) {
                return Err(bad(lang));
            }
            if self.ctx.dry_run {
                return Ok(json!({"ok": true, "dryRun": true}));
            }
            if found.active_profile.as_deref() != Some(profile.as_str()) {
                let (cn, pn) = (card.clone(), profile.clone());
                c.write(move |e| Ok(vec![e.set_profile(&cn, &pn)?]))
                    .map_err(|f| fail(f, lang))?;
                let index = found.index;
                let until = Instant::now() + PROFILE_WAIT;
                name = loop {
                    let hit = c.snapshot().and_then(|m| device_of(&m, index));
                    if hit.is_some() || Instant::now() >= until {
                        break hit;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                };
                if name.is_none() {
                    return Err(ApiError::tr(
                        "failed",
                        lang,
                        "sound.profileTimeout",
                        NO_PARAMS,
                    ));
                }
            }
        }
        let name = match name {
            Some(n) => n,
            // the device of the card's port in the active profile
            None => {
                let card = card
                    .as_ref()
                    .and_then(|n| snapshot.card_named(n))
                    .ok_or_else(|| bad(lang))?;
                device_of(&snapshot, card.index).ok_or_else(|| fail(Fail::Gone, lang))?
            }
        };
        let write = !self.ctx.dry_run;
        let device = name.clone();
        c.write(move |e| {
            let d = e.device(sink, Some(&device), None)?;
            if d.monitor_of.is_some() {
                return Err(Fail::Bad);
            }
            let mut out = Vec::new();
            if let Some(p) = &port {
                if !d.ports.iter().any(|x| &x.name == p) {
                    return Err(Fail::Bad);
                }
                if write && d.active_port.as_deref() != Some(p.as_str()) {
                    out.push(e.set_port(&d, sink, p)?);
                }
            }
            if write {
                out.push(e.set_default(sink, &device)?);
            }
            Ok(out)
        })
        .map_err(|f| fail(f, lang))?;
        let mut out = json!({"ok": true, "device": name});
        if !write {
            out["dryRun"] = json!(true);
        }
        Ok(out)
    }

    /// sound.test {sink?, channel?, alert?}: a speaker test on one channel of a sink (GNOME's Test window), or an
    /// alert of the Geluidsschema (none, default, click, string, swing, hum).
    fn test(&self, c: &Arc<Client>, args: &Value, lang: Lang) -> ApiResult {
        let snapshot = c.snapshot().ok_or_else(|| fail(Fail::Unavailable, lang))?;
        let sink = match text_arg(args, "sink") {
            Some(name) => Some(
                snapshot
                    .sink_named(name)
                    .ok_or_else(|| fail(Fail::Gone, lang))?
                    .name
                    .clone(),
            ),
            None => None,
        };
        let theme = crate::services::gsettings::read_item(alerts::SCHEMA, None, "theme-name", lang)
            .ok()
            .and_then(|i| i["value"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "freedesktop".into());
        let dirs = alerts::Dirs::system();
        let path;
        let sound = if let Some(alert) = text_arg(args, "alert") {
            match alert {
                "none" => return Ok(json!({"ok": true, "played": false})),
                "default" => canberra::Sound::Event("bell-window-system"),
                a if alerts::SOUNDS.contains(&a) => {
                    path = dirs.sound(a).to_string_lossy().into_owned();
                    canberra::Sound::File(&path)
                }
                _ => return Err(bad(lang)),
            }
        } else {
            let channel = text_arg(args, "channel")
                .filter(|ch| canberra::CHANNELS.contains(ch))
                .ok_or_else(|| bad(lang))?;
            canberra::Sound::Channel(channel)
        };
        if !canberra::available() {
            return Err(ApiError::tr(
                "unsupported",
                lang,
                "sound.testUnavailable",
                NO_PARAMS,
            ));
        }
        if !canberra::play(sink.as_deref(), &theme, sound) {
            return Err(ApiError::tr("failed", lang, "sound.testFailed", NO_PARAMS));
        }
        Ok(json!({"ok": true, "played": true}))
    }
}

// ------------------------------------------------------------------------------- system.audio.* (amendment 39)

/// The client for system.audio.*: connected, after waiting once for the first connection; None means wpctl.
fn audio_client() -> Option<Arc<Client>> {
    if !client::possible() {
        return None;
    }
    let c = client::shared()?;
    c.wait_ready(Duration::from_millis(1500)).then_some(c)
}

/// The id system.audio.* gives a sink: PipeWire's object id (what wpctl and the old payload used; Bluetooth's
/// follow-the-headset code inspects it with wpctl), else the pulse index.
fn legacy_id(d: &model::Device) -> i64 {
    d.props
        .get("object.id")
        .and_then(|s| s.parse().ok())
        .unwrap_or(d.index as i64)
}

/// system.audio.get's payload `{available, volume, muted, sinks: [{id, name, default}]}` from the model.
pub(crate) fn legacy_state(m: &model::Model) -> Value {
    let sinks: Vec<Value> = m
        .sinks
        .values()
        .map(|d| json!({"id": legacy_id(d), "name": d.description, "default": d.name == m.server.default_sink}))
        .collect();
    match m.default_sink() {
        Some(d) => {
            json!({"available": true, "volume": model::ui_pct(&d.volume), "muted": d.mute, "sinks": sinks})
        }
        None => json!({"available": false, "volume": 0, "muted": false, "sinks": sinks}),
    }
}

/// Whether system.audio.* runs on the PulseAudio client now.
pub(crate) fn audio_ready() -> bool {
    client::possible() && client::shared().is_some_and(|c| c.ready())
}

/// Whether a pulse server answers the client (waits at most 1.5 s for the first connection): libpulse may load where
/// no pulse server runs (PipeWire without pipewire-pulse), and system.audio's monitor then watches with pw-dump.
pub(crate) fn audio_connected() -> bool {
    audio_client().is_some()
}

/// system.audio.get through the client (None: not connected, use wpctl).
pub(crate) fn audio_state() -> Option<Value> {
    let c = audio_client()?;
    Some(legacy_state(&c.snapshot()?))
}

/// system.audio.set's volume (0-100) and mute on the default sink; the answer is the state with them (the server's
/// own change follows as an event).
pub(crate) fn audio_apply(volume: Option<i64>, muted: Option<bool>) -> Option<Result<Value, Fail>> {
    let c = audio_client()?;
    let mut state = legacy_state(&c.snapshot()?);
    let r = c.write(move |e| {
        let name = e.model().server.default_sink.clone();
        let d = e.device(true, Some(&name), None)?;
        let mut out = Vec::new();
        if let Some(v) = volume {
            out.push(e.set_device_volume(
                &d,
                true,
                e.scaled(&d.volume, model::from_pct(v.clamp(0, 100) as f64)),
            ));
        }
        if let Some(m) = muted {
            out.push(e.set_device_mute(&d, true, m));
        }
        Ok(out)
    });
    Some(r.map(|_| {
        if let Some(v) = volume {
            state["volume"] = json!(v.clamp(0, 100));
        }
        if let Some(m) = muted {
            state["muted"] = json!(m);
        }
        state
    }))
}

/// system.audio.setSink {id}: the sink with that id becomes the default.
pub(crate) fn audio_set_sink(id: i64) -> Option<Result<Value, Fail>> {
    let c = audio_client()?;
    let m = c.snapshot()?;
    let Some(d) = m.sinks.values().find(|d| legacy_id(d) == id) else {
        return Some(Err(Fail::Gone));
    };
    let name = d.name.clone();
    let mut state = legacy_state(&m);
    let target = name.clone();
    Some(
        c.write(move |e| Ok(vec![e.set_default(true, &target)?]))
            .map(|_| {
                if let Some(list) = state["sinks"].as_array_mut() {
                    for s in list {
                        s["default"] = json!(s["id"] == id);
                    }
                }
                if let Some(d) = m.sink_named(&name) {
                    state["volume"] = json!(model::ui_pct(&d.volume));
                    state["muted"] = json!(d.mute);
                    state["available"] = json!(true);
                }
                state
            }),
    )
}

/// system.audio's change monitor on the client's events (system/audio_monitor.rs); None without libpulse.
pub(crate) fn audio_listen(changed: Arc<dyn Fn() + Send + Sync>) -> Option<u64> {
    if !client::possible() {
        return None;
    }
    let c = client::shared()?;
    Some(c.listen(Arc::new(move |e| {
        if *e == Event::Changed {
            changed();
        }
    })))
}

pub(crate) fn audio_unlisten(id: u64) {
    if let Some(c) = client::shared() {
        c.unlisten(id);
    }
}
