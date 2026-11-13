use super::{bool_arg_exact, common::*, number_arg, pct_arg};
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::{sync::Mutex, thread, time::Duration};
pub(super) struct Audio {
    cache: Mutex<(String, Vec<Value>)>,
}
impl Audio {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new((String::new(), vec![])),
        }
    }
    pub fn get(&self, fresh: bool) -> Value {
        let none = || json!({"available":false,"volume":0,"muted":false,"sinks":[]});
        if !available("wpctl") {
            return none();
        }
        let r = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], 2, false);
        let Some((volume, muted)) = parse_volume(&r.out).filter(|_| r.ok) else {
            return none();
        };
        let sinks = if fresh {
            self.sinks()
        } else {
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .1
                .clone()
        };
        json!({"available":true,"volume":volume,"muted":muted,"sinks":sinks})
    }
    pub fn sinks(&self) -> Vec<Value> {
        let r = run(&["wpctl", "status"], 3, false);
        if !r.ok {
            return vec![];
        }
        let (mut sinks, filter) = parse_status(&r.out);
        let sig = json!([sinks, filter]).to_string();
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if cache.0 == sig && !cache.1.is_empty() {
                return cache.1.clone();
            }
        }
        if !sinks.is_empty() && !sinks.iter().any(|s| s["default"] == true) {
            let ins = run(&["wpctl", "inspect", "@DEFAULT_AUDIO_SINK@"], 2, false);
            if ins.ok {
                let desc = property(&ins.out, "node.description").unwrap_or_default();
                let id = ins
                    .out
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("id ")
                            .and_then(|s| s.split_whitespace().next())
                            .and_then(|s| s.parse::<i64>().ok())
                    })
                    .unwrap_or(-1);
                if let Some(s) = sinks.iter_mut().find(|s| s["id"] == id) {
                    s["default"] = json!(true)
                } else if !desc.is_empty() {
                    if let Some(s) = sinks.iter_mut().find(|s| {
                        string(&s["name"]) == desc || string(&s["name"]).starts_with(&desc)
                    }) {
                        s["default"] = json!(true);
                    }
                }
            }
        }
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = (sig, sinks.clone());
        sinks
    }
    pub fn set(&self, args: &Value, lang: Lang) -> ApiResult {
        let cur = self.get(false);
        if cur["available"] != true {
            return Err(ApiError::tr(
                "unavailable",
                lang,
                "system.audio.noDevice",
                crate::i18n::NO_PARAMS,
            ));
        }
        let mut volume = args["volume"].clone();
        let delta = &args["delta"];
        let mut muted = args["muted"].clone();
        if !delta.is_null() {
            volume = json!(number(&cur["volume"]).unwrap_or(0.) + number_arg(delta, lang)?)
        }
        if let Some(s) = muted.as_str() {
            if s.eq_ignore_ascii_case("toggle") {
                muted = json!(!truth(&cur["muted"]));
            } else {
                muted = json!(bool_arg_exact(&muted, lang)?);
            }
        } else if !muted.is_null() {
            muted = json!(bool_arg_exact(&muted, lang)?)
        }
        if !volume.is_null() {
            let v = if delta.is_null() {
                pct_arg(&volume, lang)?
            } else {
                round(number_arg(&volume, lang)?, 0).clamp(0., 100.) as i64
            };
            let s = format!("{:.2}", v as f64 / 100.);
            if !run(
                &[
                    "wpctl",
                    "set-volume",
                    "-l",
                    "1.0",
                    "@DEFAULT_AUDIO_SINK@",
                    &s,
                ],
                3,
                false,
            )
            .ok
            {
                return Err(ApiError::tr(
                    "failed",
                    lang,
                    "system.audio.volumeFailed",
                    crate::i18n::NO_PARAMS,
                ));
            }
            if muted.is_null() && truth(&cur["muted"]) && v > 0 {
                muted = json!(false)
            }
        }
        if !muted.is_null()
            && !run(
                &[
                    "wpctl",
                    "set-mute",
                    "@DEFAULT_AUDIO_SINK@",
                    if truth(&muted) { "1" } else { "0" },
                ],
                3,
                false,
            )
            .ok
        {
            return Err(ApiError::tr(
                "failed",
                lang,
                "system.audio.muteFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        Ok(self.get(false))
    }
    pub fn set_sink(&self, id: &Value, lang: Lang) -> ApiResult {
        let id = integer(id).ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                lang,
                "system.audio.unknownDevice",
                crate::i18n::NO_PARAMS,
            )
        })?;
        if !self.sinks().iter().any(|s| s["id"] == id) {
            return Err(ApiError::tr(
                "not_found",
                lang,
                "system.audio.deviceGone",
                crate::i18n::NO_PARAMS,
            ));
        }
        let target = self.default_target(id);
        if !run(&["wpctl", "set-default", &target.to_string()], 3, false).ok {
            return Err(ApiError::tr(
                "failed",
                lang,
                "system.audio.switchFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        thread::sleep(Duration::from_millis(150));
        Ok(self.get(true))
    }
    pub fn default_target(&self, id: i64) -> i64 {
        let ins = run(&["wpctl", "inspect", &id.to_string()], 2, false);
        if !ins.ok || property(&ins.out, "media.class").as_deref() != Some("Audio/Sink/Internal") {
            return id;
        }
        let dev = property(&ins.out, "device.id");
        let st = run(&["wpctl", "status"], 3, false);
        for fid in parse_filters(&st.out) {
            let fi = run(&["wpctl", "inspect", &fid.to_string()], 2, false);
            if fi.ok
                && property(&fi.out, "media.class").as_deref() == Some("Audio/Sink")
                && (property(&fi.out, "node.driver-id") == Some(id.to_string())
                    || (dev.is_some() && property(&fi.out, "device.id") == dev))
            {
                return fid;
            }
        }
        id
    }
}
pub(super) fn parse_volume(s: &str) -> Option<(i64, bool)> {
    let r = s.split("Volume:").nth(1)?.trim_start();
    let value = r.split_whitespace().next()?.parse::<f64>().ok()?;
    Some((round(value * 100., 0) as i64, r.contains("[MUTED]")))
}
fn clean(line: &str) -> &str {
    line.trim_start_matches(|c: char| c.is_whitespace() || "│├└─".contains(c))
}
fn parse_entry(s: &str) -> Option<(bool, i64, String, String)> {
    let mut body = clean(s);
    let star = body.starts_with('*');
    if star {
        body = body[1..].trim_start()
    }
    let (id, rest) = body.split_once('.')?;
    if !id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let id = id.parse().ok()?;
    let rest = rest.trim();
    let (name, tag) = if rest.ends_with(']') {
        if let Some(i) = rest.rfind('[') {
            (rest[..i].trim(), &rest[i..])
        } else {
            (rest, "")
        }
    } else {
        (rest, "")
    };
    Some((star, id, name.into(), tag.into()))
}
pub(super) fn parse_status(s: &str) -> (Vec<Value>, Option<i64>) {
    let mut audio = false;
    let mut section = "";
    let mut sinks = vec![];
    let mut default = None;
    for raw in s.lines() {
        let line = raw.trim();
        if ["Audio", "Video", "Settings"].contains(&line) {
            audio = line == "Audio";
            section = "";
            continue;
        }
        if !audio {
            continue;
        }
        let body = clean(raw);
        if ["Devices:", "Sinks:", "Sources:", "Filters:", "Streams:"].contains(&body) {
            section = body.trim_end_matches(':');
            continue;
        }
        if let Some((star, id, name, tag)) = parse_entry(raw) {
            if section == "Sinks" {
                sinks.push(json!({"id":id,"name":name,"default":star}));
            } else if section == "Filters" && star && tag == "[Audio/Sink]" {
                default = Some(id)
            }
        }
    }
    (sinks, default)
}
fn parse_filters(s: &str) -> Vec<i64> {
    let mut audio = false;
    let mut filters = false;
    let mut out = vec![];
    for raw in s.lines() {
        let body = clean(raw);
        if ["Audio", "Video", "Settings"].contains(&body) {
            audio = body == "Audio";
            filters = false;
            continue;
        }
        if body.ends_with(':') {
            filters = body == "Filters:";
            continue;
        }
        if audio && filters {
            if let Some((_, id, _, tag)) = parse_entry(raw) {
                if tag == "[Audio/Sink]" {
                    out.push(id)
                }
            }
        }
    }
    out
}
pub(super) fn property(s: &str, key: &str) -> Option<String> {
    for line in s.lines() {
        let t = line.trim_start().trim_start_matches('*').trim_start();
        if let Some((name, value)) = t.split_once('=') {
            if name.trim() == key {
                let value = value.trim_start();
                if let Some(value) = value.strip_prefix('"') {
                    return Some(value.split('"').next().unwrap_or("").into());
                }
            }
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wpctl_tree_and_filter() {
        let s = "Audio\n ├─ Sinks:\n │ * 42. Speaker [vol: 0.50]\n │ 44. Another\n ├─ Filters:\n │ * 50. Bluetooth [Audio/Sink]\nVideo\n ├─ Sinks:\n 99. Wrong";
        assert_eq!(
            parse_status(s),
            (
                vec![
                    json!({"id":42,"name":"Speaker","default":true}),
                    json!({"id":44,"name":"Another","default":false})
                ],
                Some(50)
            )
        );
        assert_eq!(parse_filters(s), vec![50]);
        assert_eq!(parse_volume("Volume: 0.625 [MUTED]"), Some((62, true)));
    }
    #[test]
    fn inspect_properties() {
        assert_eq!(
            property(" * node.description = \"Hello\"\n", "node.description"),
            Some("Hello".into())
        );
    }
}
