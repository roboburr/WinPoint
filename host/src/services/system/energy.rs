//! Native GNOME energy preferences. GNOME remains the owner of idle/suspend policy;
//! this module changes the same GSettings keys and power-profile property as Settings.
use super::common::{call, first, getall, setprop, system};
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use gio::prelude::*;
use glib::variant::ToVariant;
use serde_json::{Value, json};

const SESSION_SCHEMA: &str = "org.gnome.desktop.session";
const POWER_SCHEMA: &str = "org.gnome.settings-daemon.plugins.power";
pub(super) const PROFILE_ENDPOINTS: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

pub(super) struct Preferences {
    session: Option<gio::Settings>,
    power: Option<gio::Settings>,
}
impl Preferences {
    pub fn new() -> Self {
        Self::with_source(gio::SettingsSchemaSource::default().as_ref(), None)
    }
    fn with_source(
        source: Option<&gio::SettingsSchemaSource>,
        backend: Option<&gio::SettingsBackend>,
    ) -> Self {
        let make = |id| {
            source
                .and_then(|s| s.lookup(id, true))
                .map(|schema| gio::Settings::new_full(&schema, backend, None))
        };
        Self {
            session: make(SESSION_SCHEMA),
            power: make(POWER_SCHEMA),
        }
    }
    /// Keep these settings alive on the service's existing GLib monitor context.
    /// No idle polling process/timer is needed for either value or policy-lock changes.
    pub fn watch(&self, dirty: impl Fn() + Clone + 'static) {
        for settings in [&self.session, &self.power].into_iter().flatten() {
            let notify = dirty.clone();
            settings.connect_changed(None, move |_, key| {
                if is_energy_key(key) {
                    notify();
                }
            });
            let notify = dirty.clone();
            settings.connect_writable_changed(None, move |_, key| {
                if is_energy_key(key) {
                    notify();
                }
            });
        }
    }
    pub fn snapshot(&self, bus: Option<&gio::DBusConnection>) -> Value {
        let mut screen = json!({"available":false,"writable":false,"idleDelay":null});
        if let Some(s) = key(self.session.as_ref(), "idle-delay", "u") {
            screen = json!({"available":true,"writable":s.is_writable("idle-delay"),"idleDelay":s.uint("idle-delay")});
        }
        let mut dim = json!({"available":false,"writable":false,"enabled":null});
        if let Some(s) = key(self.power.as_ref(), "idle-dim", "b") {
            dim = json!({"available":true,"writable":s.is_writable("idle-dim"),"enabled":s.boolean("idle-dim")});
        }
        json!({"screen":screen,"sleepAC":self.sleep("ac"),"sleepBattery":self.sleep("battery"),"dim":dim,
            "profiles":profile_state(bus).map(|(_, value)|value).unwrap_or_else(||json!({"available":false,"writable":false,"active":null,"choices":[],"degraded":"","inhibited":"","reason":"unavailable"})),
            "canSuspend":can_suspend(bus)})
    }
    fn sleep(&self, source: &str) -> Value {
        let action = format!("sleep-inactive-{source}-type");
        let timeout = format!("sleep-inactive-{source}-timeout");
        let (Some(s), Some(_)) = (
            key(self.power.as_ref(), &action, "s"),
            key(self.power.as_ref(), &timeout, "i"),
        ) else {
            return json!({"available":false,"writable":false,"action":null,"timeout":null});
        };
        json!({"available":true,"writable":s.is_writable(&action)&&s.is_writable(&timeout),"action":s.string(&action).to_string(),"timeout":s.int(&timeout)})
    }
    pub fn set(&self, args: &Value, bus: Option<&gio::DBusConnection>, lang: Lang) -> ApiResult {
        let patch = Patch::parse(args, lang)?;
        match patch {
            Patch::IdleDelay(value) => write(
                self.session.as_ref(),
                &[("idle-delay", value.to_variant())],
                lang,
            )?,
            Patch::IdleDim(value) => write(
                self.power.as_ref(),
                &[("idle-dim", value.to_variant())],
                lang,
            )?,
            Patch::Sleep {
                battery,
                action,
                timeout,
            } => {
                if action == "suspend" && !can_suspend(bus) {
                    return Err(ApiError::tr(
                        "unsupported",
                        lang,
                        "system.energy.noSleep",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                let source = if battery { "battery" } else { "ac" };
                write(
                    self.power.as_ref(),
                    &[
                        (
                            &format!("sleep-inactive-{source}-type"),
                            action.to_variant(),
                        ),
                        (
                            &format!("sleep-inactive-{source}-timeout"),
                            timeout.to_variant(),
                        ),
                    ],
                    lang,
                )?;
            }
            Patch::Profile(id) => {
                let Some(bus) = bus else {
                    return Err(unavailable(lang));
                };
                let Some(((name, path), state)) = profile_state(Some(bus)) else {
                    return Err(unavailable(lang));
                };
                let choice = state["choices"]
                    .as_array()
                    .and_then(|choices| choices.iter().find(|c| c["id"] == id))
                    .ok_or_else(|| {
                        ApiError::tr(
                            "bad_args",
                            lang,
                            "system.energy.profileUnavailable",
                            crate::i18n::NO_PARAMS,
                        )
                    })?;
                if choice["disabled"] == true {
                    return Err(ApiError::tr(
                        "unsupported",
                        lang,
                        "system.energy.profileDisabled",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                setprop(
                    bus,
                    name,
                    path,
                    name,
                    "ActiveProfile",
                    id.to_variant(),
                    15000,
                )
                .map_err(|message| profile_error(message, lang))?;
                // Read the selected endpoint back. Never claim success after a stale,
                // unavailable or rejected write, and do not silently write a fallback daemon.
                let updated = read_profile(bus, name, path).ok_or_else(|| {
                    ApiError::tr(
                        "failed",
                        lang,
                        "system.energy.profileUnchecked",
                        crate::i18n::NO_PARAMS,
                    )
                })?;
                if updated["active"] != id {
                    return Err(ApiError::tr(
                        "failed",
                        lang,
                        "system.energy.profileNotTaken",
                        crate::i18n::NO_PARAMS,
                    ));
                }
            }
        }
        Ok(self.snapshot(bus))
    }
}
fn is_energy_key(key: &str) -> bool {
    matches!(
        key,
        "idle-delay"
            | "idle-dim"
            | "sleep-inactive-ac-type"
            | "sleep-inactive-ac-timeout"
            | "sleep-inactive-battery-type"
            | "sleep-inactive-battery-timeout"
    )
}
fn key<'a>(settings: Option<&'a gio::Settings>, name: &str, ty: &str) -> Option<&'a gio::Settings> {
    let s = settings?;
    let schema = s.settings_schema()?;
    (schema.has_key(name) && schema.key(name).value_type().as_str() == ty).then_some(s)
}
fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsupported",
        lang,
        "system.energy.unavailable",
        crate::i18n::NO_PARAMS,
    )
}
fn write(
    settings: Option<&gio::Settings>,
    values: &[(&str, glib::Variant)],
    lang: Lang,
) -> Result<(), ApiError> {
    let Some(settings) = settings else {
        return Err(unavailable(lang));
    };
    let Some(schema) = settings.settings_schema() else {
        return Err(unavailable(lang));
    };
    // Validate every key before queuing any write. In particular a locked timeout
    // cannot leave the corresponding action changed on its own.
    for (name, value) in values {
        if !schema.has_key(name) {
            return Err(unavailable(lang));
        }
        let k = schema.key(name);
        if k.value_type() != value.type_() || !k.range_check(value) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "system.energy.badValue",
                crate::i18n::NO_PARAMS,
            ));
        }
        if !settings.is_writable(name) {
            return Err(ApiError::tr(
                "denied",
                lang,
                "system.energy.locked",
                crate::i18n::NO_PARAMS,
            ));
        }
    }
    settings.delay();
    for (name, value) in values {
        if settings.set_value(name, value).is_err() {
            settings.revert();
            return Err(ApiError::tr(
                "failed",
                lang,
                "system.energy.saveFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
    }
    settings.apply();
    gio::Settings::sync();
    if values
        .iter()
        .any(|(name, value)| settings.value(name) != *value)
    {
        return Err(ApiError::tr(
            "failed",
            lang,
            "system.energy.notTaken",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(())
}
enum Patch {
    IdleDelay(u32),
    IdleDim(bool),
    Sleep {
        battery: bool,
        action: String,
        timeout: i32,
    },
    Profile(String),
}
impl Patch {
    fn parse(args: &Value, lang: Lang) -> Result<Self, ApiError> {
        let bad = || {
            ApiError::tr(
                "bad_args",
                lang,
                "system.energy.badPatch",
                crate::i18n::NO_PARAMS,
            )
        };
        let object = args.as_object().filter(|o| o.len() == 1).ok_or_else(bad)?;
        let (name, value) = object.iter().next().unwrap();
        Ok(match name.as_str() {
            "idleDelay" => Self::IdleDelay(
                value
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(bad)?,
            ),
            "idleDim" => Self::IdleDim(value.as_bool().ok_or_else(bad)?),
            "sleepAC" | "sleepBattery" => {
                let fields = value
                    .as_object()
                    .filter(|o| {
                        o.len() == 2 && o.contains_key("action") && o.contains_key("timeout")
                    })
                    .ok_or_else(bad)?;
                let action = fields["action"]
                    .as_str()
                    .filter(|s| matches!(*s, "nothing" | "suspend"))
                    .ok_or_else(bad)?;
                let timeout = fields["timeout"]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .filter(|v| *v >= 0 && (action == "nothing" || *v > 0))
                    .ok_or_else(bad)?;
                Self::Sleep {
                    battery: name == "sleepBattery",
                    action: action.into(),
                    timeout,
                }
            }
            "powerProfile" => Self::Profile(
                value
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128)
                    .ok_or_else(bad)?
                    .into(),
            ),
            _ => return Err(bad()),
        })
    }
}
fn read_profile(bus: &gio::DBusConnection, name: &str, path: &str) -> Option<Value> {
    let properties = getall(bus, name, path, name).ok()?;
    let active = properties["ActiveProfile"].as_str()?;
    let inhibited = properties["PerformanceInhibited"].as_str().unwrap_or("");
    let mut choices = vec![];
    for profile in properties["Profiles"].as_array()? {
        let Some(id) = profile["Profile"]
            .as_str()
            .filter(|p| !p.is_empty() && p.len() <= 128)
        else {
            continue;
        };
        if !choices.iter().any(|p: &Value| p["id"] == id) {
            choices.push(json!({"id":id,"disabled":id=="performance" && !inhibited.is_empty()}));
        }
    }
    if choices.is_empty() || !choices.iter().any(|p| p["id"] == active) {
        return None;
    }
    Some(
        json!({"available":true,"writable":true,"active":active,"choices":choices,
        "degraded":properties["PerformanceDegraded"].as_str().unwrap_or(""),"inhibited":inhibited}),
    )
}
fn profile_state(
    bus: Option<&gio::DBusConnection>,
) -> Option<((&'static str, &'static str), Value)> {
    let bus = bus?;
    PROFILE_ENDPOINTS
        .iter()
        .find_map(|&(name, path)| read_profile(bus, name, path).map(|state| ((name, path), state)))
}
fn can_suspend(bus: Option<&gio::DBusConnection>) -> bool {
    bus.and_then(|bus| {
        call(
            bus,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "CanSuspend",
            None,
            1500,
        )
        .ok()
    })
    .is_some_and(|reply| matches!(first(&reply).as_str(), Some("yes" | "challenge")))
}
fn profile_error(message: String, lang: Lang) -> ApiError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("accessdenied")
        || lower.contains("notauthorized")
        || lower.contains("permission")
        || lower.contains("auth")
    {
        ApiError::tr(
            "denied",
            lang,
            "system.energy.profileDenied",
            crate::i18n::NO_PARAMS,
        )
    } else {
        ApiError::tr(
            "failed",
            lang,
            "system.energy.profileFailed",
            crate::i18n::NO_PARAMS,
        )
    }
}
pub(super) fn validate(args: &Value, lang: Lang) -> Result<(), ApiError> {
    Patch::parse(args, lang).map(|_| ())
}
pub(super) fn get() -> Value {
    Preferences::new().snapshot(system().ok().as_ref())
}
pub(super) fn set(args: &Value, lang: Lang) -> ApiResult {
    Preferences::new().set(args, system().ok().as_ref(), lang)
}

#[cfg(test)]
#[path = "energy/tests.rs"]
mod tests;
