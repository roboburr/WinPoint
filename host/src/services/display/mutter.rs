//! GNOME's displays through Mutter's DisplayConfig on the session bus (notes/claude/settings-parity.md §5.5,
//! amendments 19, 25, 31-33, 64, 69): GetCurrentState decoded for Settings' Beeldscherm pages, ApplyMonitorsConfig
//! encoded from what they send, the keep-changes question through extension/displayBridge.js, and gsd-color's night
//! light. The signatures are libmutter-18's (amendment 33): GetCurrentState
//! `(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})`, ApplyMonitorsConfig `(uua(iiduba(ssa{sv}))a{sv})`.
//! No DisplayConfig XML is installed, so the tests use fixtures and a fake on the private session bus.
use crate::files::ApiError;
use crate::services::system::common::{
    call, getall, integer, number, remote_error_name, round, string, truth, variant_json,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};

pub(crate) const NAME: &str = "org.gnome.Mutter.DisplayConfig";
pub(crate) const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
pub(crate) const IFACE: &str = "org.gnome.Mutter.DisplayConfig";
pub(crate) const COLOR_NAME: &str = "org.gnome.SettingsDaemon.Color";
pub(crate) const COLOR_PATH: &str = "/org/gnome/SettingsDaemon/Color";
pub(crate) const COLOR_IFACE: &str = "org.gnome.SettingsDaemon.Color";

/// ApplyMonitorsConfig's methods (mutter's MetaMonitorsConfigMethod).
pub(crate) const VERIFY: u32 = 0;
pub(crate) const TEMPORARY: u32 = 1;
pub(crate) const PERSISTENT: u32 = 2;

/// How long Settings' "Deze beeldscherminstellingen behouden?" counts down (Windows: 15 s; mutter itself reverts
/// after its own 20 s when nobody answers).
pub(crate) const KEEP_SECONDS: u32 = 15;

fn bad(lang: crate::i18n::Lang) -> ApiError {
    ApiError::tr(
        "bad_args",
        lang,
        "display.badConfig",
        crate::i18n::NO_PARAMS,
    )
}

/// The monitors and logical monitors of one GetCurrentState answer (its `variant_json` form: `[serial, monitors,
/// logical_monitors, properties]`) with DisplayConfig's D-Bus properties (`props`, GetAll) as display.state answers
/// them.
pub(crate) fn decode_state(reply: &Value, props: &Value) -> Value {
    let logical: Vec<Value> = reply[2]
        .as_array()
        .into_iter()
        .flatten()
        .map(|lm| {
            let monitors: Vec<Value> = lm[5]
                .as_array()
                .into_iter()
                .flatten()
                .map(|spec| json!(string(&spec[0])))
                .collect();
            json!({
                "x": integer(&lm[0]).unwrap_or(0),
                "y": integer(&lm[1]).unwrap_or(0),
                "scale": number(&lm[2]).unwrap_or(1.),
                "transform": integer(&lm[3]).unwrap_or(0),
                "primary": truth(&lm[4]),
                "monitors": monitors,
            })
        })
        .collect();
    let mut monitors = Vec::new();
    for m in reply[1].as_array().into_iter().flatten() {
        let spec = &m[0];
        let connector = string(&spec[0]);
        let p = &m[2];
        let (mut current, mut preferred) = (Value::Null, Value::Null);
        let mut modes = Vec::new();
        for md in m[1].as_array().into_iter().flatten() {
            if md.as_array().is_none_or(|a| a.len() < 7) {
                continue;
            }
            let id = string(&md[0]);
            let mp = &md[6];
            if truth(&mp["is-current"]) {
                current = json!(id);
            }
            if truth(&mp["is-preferred"]) {
                preferred = json!(id);
            }
            let scales: Vec<f64> = md[5]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(number)
                .collect();
            modes.push(json!({
                "id": id,
                "width": integer(&md[1]).unwrap_or(0),
                "height": integer(&md[2]).unwrap_or(0),
                "refresh": round(number(&md[3]).unwrap_or(0.), 3),
                "preferredScale": number(&md[4]).unwrap_or(1.),
                "scales": scales,
                "current": truth(&mp["is-current"]),
                "preferred": truth(&mp["is-preferred"]),
                "interlaced": truth(&mp["is-interlaced"]),
                "variable": mp["refresh-rate-mode"] == "variable",
            }));
        }
        let index = logical.iter().position(|l| {
            l["monitors"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| *c == connector))
        });
        let lm = index.map(|i| &logical[i]);
        let field = |key: &str| lm.map_or(Value::Null, |l| l[key].clone());
        let privacy = match p["privacy-screen-state"].as_array() {
            Some(a) if a.len() == 2 => json!({"enabled": truth(&a[0]), "locked": truth(&a[1])}),
            _ => Value::Null,
        };
        let color_modes: Vec<i64> = p["supported-color-modes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(integer)
            .collect();
        let display_name = p["display-name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| connector.clone());
        monitors.push(json!({
            "connector": connector,
            "vendor": string(&spec[1]),
            "product": string(&spec[2]),
            "serial": string(&spec[3]),
            "displayName": display_name,
            "builtin": truth(&p["is-builtin"]),
            "forLease": truth(&p["is-for-lease"]),
            // present only when the output supports underscanning (mutter's is-underscanning)
            "underscanning": p.get("is-underscanning").map(truth),
            "privacyScreen": privacy,
            "minRefreshRate": integer(&p["min-refresh-rate"]),
            "colorMode": integer(&p["color-mode"]),
            "rgbRange": integer(&p["rgb-range"]),
            "colorModes": color_modes,
            "modes": modes,
            "current": current,
            "preferred": preferred,
            "logical": index,
            "x": field("x"),
            "y": field("y"),
            "scale": field("scale"),
            "transform": field("transform"),
            "primary": lm.is_some_and(|l| l["primary"] == true),
        }));
    }
    let state = &reply[3];
    json!({
        "supported": true,
        "backend": "mutter",
        "serial": integer(&reply[0]).unwrap_or(0),
        "allowed": props["ApplyMonitorsConfigAllowed"] != false,
        "nightLightSupported": props.get("NightLightSupported").map(truth),
        "panelOrientationManaged": truth(&props["PanelOrientationManaged"]),
        "layoutMode": integer(&state["layout-mode"]).unwrap_or(1),
        "supportsChangingLayoutMode": truth(&state["supports-changing-layout-mode"]),
        "globalScaleRequired": truth(&state["global-scale-required"]),
        "monitors": monitors,
        "logical": logical,
    })
}

/// What display.apply sends to ApplyMonitorsConfig, checked.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    pub serial: u32,
    pub method: u32,
    pub logical: Vec<Logical>,
    pub layout_mode: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Logical {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: u32,
    pub primary: bool,
    pub monitors: Vec<Monitor>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Monitor {
    pub connector: String,
    pub mode: String,
    /// underscanning (b), color-mode (u), rgb-range (u): the per-monitor properties GNOME Settings sends
    pub underscanning: Option<bool>,
    pub color_mode: Option<u32>,
    pub rgb_range: Option<u32>,
}

/// A connector or mode id: printable ASCII without spaces ("DP-1", "eDP-1", "2560x1440@59.951", "...+vrr").
fn id_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'\'' && b != b'"')
}

fn uint(v: &Value, max: u64) -> Option<u32> {
    v.as_u64().filter(|n| *n <= max).map(|n| n as u32)
}

fn int(v: &Value) -> Option<i32> {
    v.as_i64()
        .filter(|n| (-65536..=65536).contains(n))
        .map(|n| n as i32)
}

/// The method's number: "verify", "temporary", "persistent" or 0-2.
pub(crate) fn method_of(v: &Value) -> Option<u32> {
    match v {
        Value::String(s) => match s.as_str() {
            "verify" => Some(VERIFY),
            "temporary" => Some(TEMPORARY),
            "persistent" => Some(PERSISTENT),
            _ => None,
        },
        Value::Number(_) => uint(v, 2),
        _ => None,
    }
}

/// display.apply's arguments `{serial, method, logical: [{x, y, scale, transform, primary, monitors: [{connector,
/// mode, props?: {underscanning?, color-mode?, rgb-range?}}]}], props?: {layout-mode?}}` as a plan; anything else
/// (a missing field, another type, an unknown property, no monitor at all, a connector twice) is
/// `bad_args display.badConfig`.
pub(crate) fn parse_apply(args: &Value, lang: crate::i18n::Lang) -> Result<Plan, ApiError> {
    let serial = uint(&args["serial"], u32::MAX as u64).ok_or_else(|| bad(lang))?;
    let method = method_of(&args["method"]).ok_or_else(|| bad(lang))?;
    let list = args["logical"].as_array().ok_or_else(|| bad(lang))?;
    if list.is_empty() || list.len() > 16 {
        return Err(bad(lang));
    }
    let mut seen = Vec::new();
    let mut logical = Vec::new();
    for lm in list {
        if !lm.is_object() {
            return Err(bad(lang));
        }
        let scale = lm["scale"]
            .as_f64()
            .filter(|s| s.is_finite() && (0.25..=8.).contains(s))
            .ok_or_else(|| bad(lang))?;
        let transform = match &lm["transform"] {
            Value::Null => 0,
            v => uint(v, 7).ok_or_else(|| bad(lang))?,
        };
        let primary = match &lm["primary"] {
            Value::Null => false,
            Value::Bool(b) => *b,
            _ => return Err(bad(lang)),
        };
        let mons = lm["monitors"].as_array().ok_or_else(|| bad(lang))?;
        if mons.is_empty() || mons.len() > 16 {
            return Err(bad(lang));
        }
        let mut monitors = Vec::new();
        for m in mons {
            let connector = m["connector"]
                .as_str()
                .filter(|s| id_ok(s))
                .ok_or_else(|| bad(lang))?;
            let mode = m["mode"]
                .as_str()
                .filter(|s| id_ok(s))
                .ok_or_else(|| bad(lang))?;
            if seen.contains(&connector) {
                return Err(bad(lang));
            }
            seen.push(connector);
            let mut monitor = Monitor {
                connector: connector.to_owned(),
                mode: mode.to_owned(),
                underscanning: None,
                color_mode: None,
                rgb_range: None,
            };
            match &m["props"] {
                Value::Null => {}
                Value::Object(props) => {
                    for (key, value) in props {
                        match key.as_str() {
                            "underscanning" => {
                                monitor.underscanning =
                                    Some(value.as_bool().ok_or_else(|| bad(lang))?)
                            }
                            "color-mode" => {
                                monitor.color_mode = Some(uint(value, 8).ok_or_else(|| bad(lang))?)
                            }
                            "rgb-range" => {
                                monitor.rgb_range = Some(uint(value, 8).ok_or_else(|| bad(lang))?)
                            }
                            _ => return Err(bad(lang)),
                        }
                    }
                }
                _ => return Err(bad(lang)),
            }
            monitors.push(monitor);
        }
        logical.push(Logical {
            x: int(&lm["x"]).ok_or_else(|| bad(lang))?,
            y: int(&lm["y"]).ok_or_else(|| bad(lang))?,
            scale,
            transform,
            primary,
            monitors,
        });
    }
    let layout_mode = match &args["props"] {
        Value::Null => None,
        Value::Object(props) => {
            let mut mode = None;
            for (key, value) in props {
                match key.as_str() {
                    "layout-mode" => {
                        mode = Some(
                            uint(value, 2)
                                .filter(|n| *n >= 1)
                                .ok_or_else(|| bad(lang))?,
                        )
                    }
                    _ => return Err(bad(lang)),
                }
            }
            mode
        }
        _ => return Err(bad(lang)),
    };
    Ok(Plan {
        serial,
        method,
        logical,
        layout_mode,
    })
}

/// ApplyMonitorsConfig's parameters `(uua(iiduba(ssa{sv}))a{sv})` for a plan, with `method` as the method.
pub(crate) fn encode(plan: &Plan, method: u32) -> glib::Variant {
    let monitor_ty = glib::VariantTy::new("(ssa{sv})").expect("type");
    let logical_ty = glib::VariantTy::new("(iiduba(ssa{sv}))").expect("type");
    let logical = plan.logical.iter().map(|l| {
        let monitors = l.monitors.iter().map(|m| {
            let props = glib::VariantDict::new(None);
            if let Some(on) = m.underscanning {
                props.insert_value("underscanning", &on.to_variant());
            }
            if let Some(mode) = m.color_mode {
                props.insert_value("color-mode", &mode.to_variant());
            }
            if let Some(range) = m.rgb_range {
                props.insert_value("rgb-range", &range.to_variant());
            }
            glib::Variant::tuple_from_iter([
                m.connector.to_variant(),
                m.mode.to_variant(),
                props.end(),
            ])
        });
        glib::Variant::tuple_from_iter([
            l.x.to_variant(),
            l.y.to_variant(),
            l.scale.to_variant(),
            l.transform.to_variant(),
            l.primary.to_variant(),
            glib::Variant::array_from_iter_with_type(monitor_ty, monitors),
        ])
    });
    let props = glib::VariantDict::new(None);
    if let Some(mode) = plan.layout_mode {
        props.insert_value("layout-mode", &mode.to_variant());
    }
    glib::Variant::tuple_from_iter([
        plan.serial.to_variant(),
        method.to_variant(),
        glib::Variant::array_from_iter_with_type(logical_ty, logical),
        props.end(),
    ])
}

/// The ApiError of a refused ApplyMonitorsConfig: a stale serial (the displays changed meanwhile), "not allowed"
/// (ApplyMonitorsConfigAllowed false: a lock screen, a remote session), anything else with mutter's message.
pub(crate) fn apply_error(text: &str, lang: crate::i18n::Lang) -> ApiError {
    let lower = text.to_ascii_lowercase();
    if lower.contains("stale") || lower.contains("serial") {
        return ApiError::tr("failed", lang, "display.stale", crate::i18n::NO_PARAMS);
    }
    if lower.contains("not allowed") {
        return ApiError::tr("denied", lang, "display.notAllowed", crate::i18n::NO_PARAMS);
    }
    let name = remote_error_name(text).unwrap_or("");
    if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
        return unavailable(lang);
    }
    let message = text.strip_prefix("GDBus.Error:").map_or(text, |rest| {
        rest.split_once(": ").map_or(rest, |(_, message)| message)
    });
    ApiError::tr(
        "failed",
        lang,
        "display.applyFailed",
        &json!({"error": message.chars().take(300).collect::<String>()}),
    )
}

pub(crate) fn unavailable(lang: crate::i18n::Lang) -> ApiError {
    ApiError::tr(
        "unsupported",
        lang,
        "display.unavailable",
        crate::i18n::NO_PARAMS,
    )
}

/// Where the services are: GNOME's own names, or a test's fakes (their unique names on the private bus).
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    pub mutter: String,
    pub color: String,
}

impl Endpoints {
    pub(crate) fn gnome() -> Self {
        Self {
            mutter: NAME.into(),
            color: COLOR_NAME.into(),
        }
    }
}

/// GetCurrentState and the interface's properties; None without DisplayConfig on the bus.
pub(crate) fn read_state(bus: &gio::DBusConnection, ep: &Endpoints) -> Option<Value> {
    let reply = call(bus, &ep.mutter, PATH, IFACE, "GetCurrentState", None, 3000).ok()?;
    let props = getall(bus, &ep.mutter, PATH, IFACE).unwrap_or(Value::Null);
    Some(decode_state(&variant_json(&reply), &props))
}

/// One ApplyMonitorsConfig call.
pub(crate) fn apply(
    bus: &gio::DBusConnection,
    ep: &Endpoints,
    plan: &Plan,
    method: u32,
    lang: crate::i18n::Lang,
) -> Result<(), ApiError> {
    call(
        bus,
        &ep.mutter,
        PATH,
        IFACE,
        "ApplyMonitorsConfig",
        Some(&encode(plan, method)),
        10_000,
    )
    .map(|_| ())
    .map_err(|e| apply_error(&e, lang))
}

/// gsd-color's night light (`display.nightLight.status`). `supported` is GNOME Settings 50's test: mutter's
/// NightLightSupported (false: "Night Light Unavailable", the hardware has no gamma); without gsd-color (`available`
/// false) the keys still work as in GNOME Settings, only Nu uitschakelen / Nu inschakelen have nothing to ask.
pub(crate) fn night_light(color: Option<&Value>, night_light_supported: Option<bool>) -> Value {
    let supported = night_light_supported != Some(false);
    let Some(p) = color else {
        return json!({"available": false, "supported": supported, "active": false,
            "disabledUntilTomorrow": false, "temperature": null, "sunrise": null, "sunset": null});
    };
    let hours = |v: &Value| number(v).filter(|h| (0. ..=24.).contains(h));
    json!({
        "available": true,
        "supported": supported,
        "active": truth(&p["NightLightActive"]),
        "disabledUntilTomorrow": truth(&p["DisabledUntilTomorrow"]),
        "temperature": integer(&p["Temperature"]),
        "sunrise": hours(&p["Sunrise"]),
        "sunset": hours(&p["Sunset"]),
    })
}

/// The night light's state from the bus.
pub(crate) fn read_night_light(bus: &gio::DBusConnection, ep: &Endpoints) -> Value {
    let color = getall(bus, &ep.color, COLOR_PATH, COLOR_IFACE).ok();
    let supported = getall(bus, &ep.mutter, PATH, IFACE)
        .ok()
        .and_then(|p| p.get("NightLightSupported").map(truth));
    night_light(color.as_ref(), supported)
}

/// The labels of Identificeren: every monitor's number in display.state's order (the numbers Settings shows).
pub(crate) fn labels(state: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (i, m) in state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(c) = m["connector"].as_str() {
            out.insert(c.to_owned(), json!(i + 1));
        }
    }
    Value::Object(out)
}
