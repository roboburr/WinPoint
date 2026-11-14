//! Display contract: Mutter's real monitor state in GNOME, native Hyprland IPC elsewhere.
use super::{ServiceContext, system::common::*};
use crate::files::{ApiError, ApiResult};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
pub(crate) struct Display {
    ctx: Arc<ServiceContext>,
    stopped: AtomicBool,
    last: Mutex<Value>,
}
impl Display {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let s = Arc::new(Self {
            ctx,
            stopped: AtomicBool::new(false),
            last: Mutex::new(Value::Null),
        });
        let weak = Arc::downgrade(&s);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(500));
                let Some(s) = weak.upgrade() else { break };
                if s.stopped.load(Ordering::Relaxed) {
                    break;
                }
                if !hypr_available() {
                    continue;
                }
                if let Some(m) = hypr_monitors() {
                    let mut last = s.last.lock().unwrap_or_else(|e| e.into_inner());
                    if !last.is_null() && *last != m {
                        s.ctx.emit("display.changed", json!({"monitors":m}));
                    }
                    *last = m;
                }
            }
        });
        Ok(s)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        match method {
            "display.list" => Some(Ok(self.list())),
            "display.set" => Some(if self.ctx.dry_run {
                Ok(json!({"ok":true,"dryRun":true,"previous":null}))
            } else {
                self.set(args)
            }),
            _ => None,
        }
    }
    fn list(&self) -> Value {
        if hypr_available() {
            if let Some(m) = hypr_monitors() {
                *self.last.lock().unwrap_or_else(|e| e.into_inner()) = m.clone();
                return json!({"supported":true,"monitors":m});
            }
        }
        json!({"supported":false,"monitors":gnome_monitors()})
    }
    fn set(&self, args: &Value) -> ApiResult {
        let name = args["name"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
            })
            .ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "display.unknown",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        if !hypr_available() {
            return Err(unsupported(self.ctx.lang()));
        }
        let mons = hypr_raw().unwrap_or_default();
        let raw = mons.iter().find(|m| m["name"] == name).ok_or_else(|| {
            ApiError::tr(
                "not_found",
                self.ctx.lang(),
                "display.gone",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let current = monitor_dict(raw);
        let mut mode = match args["mode"].as_str() {
            None | Some("") => {
                let s = string(&current["mode"]);
                if s.is_empty() { "preferred".into() } else { s }
            }
            Some(s @ ("preferred" | "highrr" | "highres")) => s.into(),
            Some(s) => norm_mode(s).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "display.badMode",
                    crate::i18n::NO_PARAMS,
                )
            })?,
        };
        if !matches!(mode.as_str(), "preferred" | "highrr" | "highres") {
            let modes = current["modes"].as_array().cloned().unwrap_or_default();
            if !modes.is_empty() && !modes.iter().any(|v| v == &mode) {
                let want = mode_key(&mode);
                mode = modes
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|m| {
                        let k = mode_key(m);
                        k.0 == want.0
                            && k.1 == want.1
                            && (want.2 == 0. || (k.2 - want.2).abs() < 0.6)
                    })
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        ApiError::tr(
                            "bad_args",
                            self.ctx.lang(),
                            "display.modeUnsupported",
                            crate::i18n::NO_PARAMS,
                        )
                    })?;
            }
        }
        let scale = if args["scale"].is_null() || args["scale"] == "" {
            number(&current["scale"]).unwrap_or(1.)
        } else {
            number(&args["scale"]).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "display.badScale",
                    crate::i18n::NO_PARAMS,
                )
            })?
        };
        if !(0.5..=4.).contains(&scale) {
            return Err(ApiError::tr(
                "bad_args",
                self.ctx.lang(),
                "display.scaleRange",
                crate::i18n::NO_PARAMS,
            ));
        }
        let scale = format!("{scale:.4}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned();
        let applied = format!("{name},{mode},{}x{},{scale}", current["x"], current["y"]);
        let reply = hypr(&format!("[[BATCH]]keyword monitor {applied}"));
        if reply
            .as_ref()
            .is_none_or(|s| s.lines().any(|l| !l.trim().is_empty() && l.trim() != "ok"))
        {
            return Err(ApiError::tr(
                "failed",
                self.ctx.lang(),
                "display.failed",
                crate::i18n::NO_PARAMS,
            ));
        }
        persist(&self.ctx, name, &applied);
        Ok(
            json!({"ok":true,"previous":{"mode":current["mode"],"scale":current["scale"]},"applied":applied}),
        )
    }
}
fn hypr_path() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })));
    let p = root.join("hypr").join(&sig).join(".socket.sock");
    if p.exists() {
        Some(p)
    } else {
        let p = PathBuf::from("/tmp/hypr").join(sig).join(".socket.sock");
        p.exists().then_some(p)
    }
}
fn hypr_available() -> bool {
    hypr_path().is_some()
}
/// D-G2: there is no Win11 session in the GNOME version, so displays are set in Ubuntu's Settings app. The text is
/// the UI's own (display.js displaysNA) in both languages; the tests compare the two.
fn unsupported(lang: crate::i18n::Lang) -> ApiError {
    ApiError::tr(
        "unsupported",
        lang,
        "display.unsupported",
        crate::i18n::NO_PARAMS,
    )
}
fn hypr(command: &str) -> Option<String> {
    let mut s = UnixStream::connect(hypr_path()?).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(command.as_bytes()).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut out = String::new();
    s.take(4 * 1024 * 1024).read_to_string(&mut out).ok()?;
    Some(out)
}
fn hypr_raw() -> Option<Vec<Value>> {
    let mut empty = None;
    for cmd in ["j/monitors all", "j/monitors"] {
        if let Some(monitors) = hypr(cmd)
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_array().cloned())
        {
            if !monitors.is_empty() {
                return Some(monitors);
            }
            empty = Some(monitors);
        }
    }
    empty
}
fn hypr_monitors() -> Option<Value> {
    hypr_raw().map(|a| {
        Value::Array(
            a.iter()
                .filter(|m| m.is_object())
                .map(monitor_dict)
                .collect(),
        )
    })
}
fn fmt_rate(v: f64) -> String {
    format!("{v:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
pub(super) fn norm_mode(text: &str) -> Option<String> {
    let decimal = |c: char| unsafe { glib::ffi::g_unichar_digit_value(c as u32) };
    let valid_dimension =
        |s: &str| (2..=5).contains(&s.chars().count()) && s.chars().all(|c| decimal(c) >= 0);
    let mut xy = text.trim().split(['x', 'X']);
    let w = xy.next()?;
    let rhs = xy.next()?;
    if xy.next().is_some() || !valid_dimension(w) {
        return None;
    }
    let mut hr = rhs.split('@');
    let h = hr.next()?;
    if !valid_dimension(h) {
        return None;
    }
    if let Some(r) = hr.next() {
        let r = if r
            .get(r.len().saturating_sub(2)..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case("hz"))
        {
            &r[..r.len() - 2]
        } else {
            r
        };
        if hr.next().is_some() || r.is_empty() || r.starts_with('.') || r.ends_with('.') {
            return None;
        }
        let mut dot = 0;
        let mut normalized = String::new();
        for c in r.chars() {
            if c == '.' {
                dot += 1;
                normalized.push(c);
            } else {
                let d = decimal(c);
                if d < 0 {
                    return None;
                }
                normalized.push(char::from(b'0' + d as u8));
            }
        }
        if dot > 1 {
            return None;
        }
        Some(format!("{w}x{h}@{}", fmt_rate(normalized.parse().ok()?)))
    } else {
        Some(format!("{w}x{h}"))
    }
}
fn mode_key(mode: &str) -> (u64, u64, f64) {
    let mut parts = mode.split(['x', '@']);
    let w = parts
        .next()
        .and_then(|v| pyint(&json!(v)).and_then(|n| u64::try_from(n).ok()))
        .unwrap_or(0);
    let h = parts
        .next()
        .and_then(|v| pyint(&json!(v)).and_then(|n| u64::try_from(n).ok()))
        .unwrap_or(0);
    let r = parts
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.);
    (w * h, w, r)
}
fn sorted_modes(modes: &mut Vec<String>) {
    modes.sort_by(|a, b| {
        let a = mode_key(a);
        let b = mode_key(b);
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.total_cmp(&a.2))
    });
    modes.dedup();
}
pub(super) fn monitor_dict(m: &Value) -> Value {
    let w = integer(&m["width"]).unwrap_or(0);
    let h = integer(&m["height"]).unwrap_or(0);
    let rate = number(&m["refreshRate"]).unwrap_or(0.);
    let scale = number(&m["scale"]).filter(|v| *v != 0.).unwrap_or(1.);
    let transform = integer(&m["transform"]).unwrap_or(0);
    let mut modes = m["availableModes"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| norm_mode(&string(v)))
        .collect::<Vec<_>>();
    sorted_modes(&mut modes);
    let mode = if w != 0 && h != 0 {
        format!("{w}x{h}@{}", fmt_rate(rate))
    } else {
        String::new()
    };
    if !mode.is_empty() && !modes.contains(&mode) {
        modes.insert(0, mode.clone());
    }
    let (lw, lh) = if transform % 2 == 1 { (h, w) } else { (w, h) };
    json!({"name":m["name"].as_str().unwrap_or(""),"description":string(&m["description"]).trim(),"make":m["make"].as_str().unwrap_or(""),"model":m["model"].as_str().unwrap_or(""),"width":w,"height":h,"refresh":round(rate,2),"scale":round(scale,4),"x":integer(&m["x"]).unwrap_or(0),"y":integer(&m["y"]).unwrap_or(0),"logicalWidth":round(lw as f64/scale,0)as i64,"logicalHeight":round(lh as f64/scale,0)as i64,"focused":truth(&m["focused"]),"transform":transform,"vrr":truth(&m["vrr"]),"disabled":truth(&m["disabled"]),"mode":mode,"modes":modes,"readOnly":false})
}
fn gnome_monitors() -> Vec<Value> {
    let Ok(bus) = session() else { return vec![] };
    let Ok(r) = call(
        &bus,
        "org.gnome.Mutter.DisplayConfig",
        "/org/gnome/Mutter/DisplayConfig",
        "org.gnome.Mutter.DisplayConfig",
        "GetCurrentState",
        None,
        2000,
    ) else {
        return vec![];
    };
    gnome_from_value(&variant_json(&r))
}
fn gnome_from_value(r: &Value) -> Vec<Value> {
    let Some(monitors) = r[1].as_array() else {
        return vec![];
    };
    let logical = r[2].as_array().cloned().unwrap_or_default();
    let mut out = vec![];
    for m in monitors {
        let spec = &m[0];
        let connector = spec[0].as_str().unwrap_or("");
        let mut modes = vec![];
        let mut current = None;
        for md in m[1].as_array().unwrap_or(&vec![]) {
            if md.as_array().is_none_or(|a| a.len() < 7) {
                continue;
            }
            let (w, h, rate) = (
                integer(&md[1]).unwrap_or(0),
                integer(&md[2]).unwrap_or(0),
                number(&md[3]).unwrap_or(0.),
            );
            let mode = format!("{w}x{h}@{}", fmt_rate(rate));
            if !modes.contains(&mode) {
                modes.push(mode);
            }
            if truth(&md[6]["is-current"]) {
                current = Some((w, h, rate));
            }
        }
        sorted_modes(&mut modes);
        let pl = logical.iter().find(|lm| {
            lm[5]
                .as_array()
                .is_some_and(|mons| mons.iter().any(|s| s == spec))
        });
        let (w, h, rate) = current.unwrap_or((0, 0, 0.));
        let scale = pl.and_then(|l| number(&l[2])).unwrap_or(1.);
        let divisor = if scale == 0. { 1. } else { scale };
        out.push(json!({"name":connector,"description":m[2]["display-name"].as_str().unwrap_or(connector),"make":spec[1].as_str().unwrap_or(""),"model":spec[2].as_str().unwrap_or(""),"width":w,"height":h,"refresh":round(rate,2),"scale":round(scale,4),"x":pl.map(|l|l[0].clone()).unwrap_or(json!(0)),"y":pl.map(|l|l[1].clone()).unwrap_or(json!(0)),"logicalWidth":round(w as f64/divisor,0)as i64,"logicalHeight":round(h as f64/divisor,0)as i64,"focused":pl.is_some_and(|l|truth(&l[4])),"transform":0,"vrr":false,"disabled":current.is_none(),"mode":if current.is_some(){format!("{w}x{h}@{}",fmt_rate(rate))}else{String::new()},"modes":modes,"readOnly":true}));
    }
    out
}
fn persist(ctx: &ServiceContext, name: &str, value: &str) {
    let p = ctx.config_dir.join("hyprland-monitors.conf");
    let mut lines = std::fs::read_to_string(&p)
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            let s = line.trim();
            !s.is_empty()
                && !(s.starts_with("monitor")
                    && s.split_once('=')
                        .is_some_and(|(_, v)| v.trim().split(',').next() == Some(name)))
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.first().is_none_or(|s| !s.starts_with('#')) {
        lines.insert(
            0,
            "# Win11 Shell: beeldscherminstellingen (automatisch bijgewerkt door Instellingen)"
                .into(),
        );
    }
    lines.push(format!("monitor = {value}"));
    let tmp = p.with_extension("conf.new");
    if std::fs::write(&tmp, lines.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_normalization() {
        assert_eq!(
            norm_mode(" 2560x1440@143.990Hz "),
            Some("2560x1440@143.99".into())
        );
        assert!(norm_mode("8x9@60").is_none());
        assert!(norm_mode("1920x1080@60;oops").is_none());
    }
    #[test]
    fn gnome_layout_contract() {
        let m = gnome_from_value(
            &json!([7,[[["HDMI-1","Make","Model","Serial"],[["id",2560,1440,59.951,1.,[1.,2.],{"is-current":true}]],{"display-name":"Screen"}]],[[12,-10,2.,3,true,[["HDMI-1","Make","Model","Serial"]],{}]],{}]),
        );
        assert_eq!(m[0]["mode"], "2560x1440@59.95");
        assert_eq!(m[0]["logicalWidth"], 1280);
        assert_eq!(m[0]["readOnly"], true);
        assert_eq!(m[0]["transform"], 0);
    }
    #[test]
    fn display_errors_render_in_the_context_language() {
        // R2-A: the display stem in English through display.set; the D-G2 text is native Settings' own notice
        for (language, unknown, not_here) in [
            (
                "en",
                "Unknown display.",
                "Display settings can’t be changed here. Use Ubuntu’s Settings app instead.",
            ),
            (
                "nl",
                "Onbekend beeldscherm.",
                "Beeldscherminstellingen wijzigen kan hier niet. Gebruik daarvoor de app Instellingen van Ubuntu.",
            ),
        ] {
            let lc = crate::services::system::common::LangContext::new(language, "settings");
            // the struct itself, without new()'s Hyprland poll thread
            let display = Display {
                ctx: lc.ctx.clone(),
                stopped: AtomicBool::new(true),
                last: Mutex::new(Value::Null),
            };
            let e = display
                .call("display.set", &json!({"name":"bad name"}))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                e.json(),
                json!({"code":"bad_args","message":unknown,"key":"display.unknown","params":{}})
            );
            // D-G2 on every machine (review m4): the error display.set returns without Hyprland ...
            let lang = lc.ctx.lang();
            assert_eq!(
                unsupported(lang).json(),
                json!({"code":"unsupported","message":not_here,"key":"display.unsupported","params":{}})
            );
            // ... is native Settings' own notice: the page's nl/en pair (display.rs render_scale)
            if language == "nl" {
                assert_eq!(
                    crate::services::system::common::native_display_text(not_here),
                    unsupported(crate::i18n::Lang::En).message
                );
            }
            // ... and display.set really returns it here (a GNOME machine has no Hyprland socket)
            if !hypr_available() {
                let e = display
                    .call("display.set", &json!({"name":"DP-1"}))
                    .unwrap()
                    .unwrap_err();
                assert_eq!(e.json(), unsupported(lang).json());
            }
        }
    }
}
