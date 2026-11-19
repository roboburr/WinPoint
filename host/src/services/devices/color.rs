//! Colour management through colord (GNOME's Color panel, cc-color-panel.c; Windows 11's Kleurprofiel): the devices
//! (displays, printers, scanners, cameras, webcams) with their profiles, the default profile, adding, importing and
//! removing profiles, "Set for all users", and display calibration with a colour sensor through colord-session's
//! org.freedesktop.ColorHelper on the session bus (cc-color-calibrate.c), whose gamma ramps go to the display through
//! Mutter's calibration session (org.gnome.Mutter.ColorManager CalibrateMonitor, then the session's
//! SetCrtcGammaLut, as gnome-control-center 50 does). colord checks polkit itself
//! (`org.freedesktop.color-manager.modify-device`, `create-profile`: YES for the active user; `install-system-wide`:
//! auth_admin_keep, §5.8), so the writes go through `call_interactive`.
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use crate::services::system::common::{
    INTERACTIVE_TIMEOUT_MS, call, call_interactive, first, getall, interactive_error, object_path,
    string, truth,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) const MANAGER_PATH: &str = "/org/freedesktop/ColorManager";
pub(super) const MANAGER: &str = "org.freedesktop.ColorManager";
const DEVICE: &str = "org.freedesktop.ColorManager.Device";
const PROFILE: &str = "org.freedesktop.ColorManager.Profile";
const SENSOR: &str = "org.freedesktop.ColorManager.Sensor";
pub(super) const HELPER_PATH: &str = "/";
pub(super) const HELPER_DISPLAY: &str = "org.freedesktop.ColorHelper.Display";
/// Mutter's colour manager (libmutter-18's introspection and gnome-control-center 50's proxy: CalibrateMonitor(s
/// connector) → (o session); the session: GammaLutSize (u), SetCrtcGammaLut(aq red, aq green, aq blue), Stop).
pub(super) const MUTTER: &str = "org.gnome.Mutter.ColorManager";
pub(super) const MUTTER_PATH: &str = "/org/gnome/Mutter/ColorManager";
pub(super) const MUTTER_CALIBRATION: &str = "org.gnome.Mutter.ColorManager.Calibration";

/// GNOME's calibration choices (cc-color-calibrate.ui): quality (CdProfileQuality), the display type (CdSensorCap)
/// and the white point (Kelvin, 0 = native).
pub(super) const QUALITIES: &[u32] = &[0, 1, 2];
pub(super) const KINDS: &[u32] = &[1, 2, 5, 8, 9, 10, 11, 12, 13, 14];
pub(super) const WHITEPOINTS: &[u32] = &[0, 5000, 5500, 6500, 7500];

pub(super) fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr(
        "unavailable",
        lang,
        "devices.noColord",
        crate::i18n::NO_PARAMS,
    )
}

fn not_found(lang: Lang) -> ApiError {
    ApiError::tr(
        "not_found",
        lang,
        "devices.colorNotFound",
        crate::i18n::NO_PARAMS,
    )
}

/// A colord object path argument (a device, a profile or a sensor).
pub(super) fn path_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|p| p.starts_with("/org/freedesktop/ColorManager/") && object_path(p).is_ok())
        .map(str::to_owned)
        .ok_or_else(|| super::bad(lang, "devices.badDevice"))
}

/// `devices.color.get` without colord.
pub(super) fn absent() -> Value {
    json!({"available": false, "devices": [], "profiles": [], "sensors": []})
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(string).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// One profile as the Color panel shows it: its title, kind, colorspace, when it was made, where it comes from
/// (metadata DATA_source: calib, edid, standard, test), whether it is installed for all users, and colord's
/// warnings about it.
pub(super) fn profile_json(path: &str, p: &Value) -> Value {
    let metadata = p["Metadata"].clone();
    json!({
        "path": path,
        "id": string(&p["ProfileId"]),
        "title": string(&p["Title"]),
        "kind": string(&p["Kind"]),
        "colorspace": string(&p["Colorspace"]),
        "qualifier": string(&p["Qualifier"]),
        "format": string(&p["Format"]),
        "filename": string(&p["Filename"]),
        "created": p["Created"].as_i64().filter(|t| *t > 0),
        "scope": string(&p["Scope"]),
        "systemWide": truth(&p["IsSystemWide"]),
        "hasVcgt": truth(&p["HasVcgt"]),
        "warnings": strings(&p["Warnings"]),
        "source": string(&metadata["DATA_source"]),
        "standardSpace": string(&metadata["STANDARD_space"]),
        "metadata": if metadata.is_object() { metadata } else { json!({}) },
    })
}

/// One device with its profiles in colord's order (the first is the default, GNOME's selected radio).
pub(super) fn device_json(path: &str, d: &Value, profiles: &BTreeMap<String, Value>) -> Value {
    let list: Vec<Value> = strings(&d["Profiles"])
        .iter()
        .map(|p| {
            profiles
                .get(p)
                .cloned()
                .unwrap_or_else(|| json!({"path": p, "title": "", "warnings": []}))
        })
        .collect();
    json!({
        "path": path,
        "id": string(&d["DeviceId"]),
        "kind": string(&d["Kind"]),
        "model": string(&d["Model"]),
        "vendor": string(&d["Vendor"]),
        "serial": string(&d["Serial"]),
        "colorspace": string(&d["Colorspace"]),
        "mode": string(&d["Mode"]),
        "scope": string(&d["Scope"]),
        "enabled": d["Enabled"].as_bool().unwrap_or(true),
        "embedded": truth(&d["Embedded"]),
        "created": d["Created"].as_u64().filter(|t| *t > 0),
        "inhibited": !strings(&d["ProfilingInhibitors"]).is_empty(),
        "default": list.first().map(|p| p["path"].clone()).unwrap_or(Value::Null),
        "profiles": list,
    })
}

fn paths(bus: &gio::DBusConnection, name: &str, method: &str) -> Vec<String> {
    call(bus, name, MANAGER_PATH, MANAGER, method, None, 3000)
        .map(|v| first(&v))
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(string)
        .collect()
}

/// `devices.color.get`: every device (displays first, as GNOME sorts them), every profile colord knows (for "Profiel
/// toevoegen") and the sensors (calibration is offered only with one).
pub(super) fn get(bus: &gio::DBusConnection, name: &str) -> Value {
    let Ok(device_paths) = call(bus, name, MANAGER_PATH, MANAGER, "GetDevices", None, 3000)
        .map(|v| first(&v))
        .map(|v| v.as_array().cloned().unwrap_or_default())
    else {
        return absent();
    };
    let mut profiles: BTreeMap<String, Value> = BTreeMap::new();
    for p in paths(bus, name, "GetProfiles") {
        if let Ok(props) = getall(bus, name, &p, PROFILE) {
            profiles.insert(p.clone(), profile_json(&p, &props));
        }
    }
    let mut devices = Vec::new();
    for p in device_paths.iter().map(string) {
        let Ok(props) = getall(bus, name, &p, DEVICE) else {
            continue;
        };
        for missing in strings(&props["Profiles"]) {
            if !profiles.contains_key(&missing)
                && let Ok(pp) = getall(bus, name, &missing, PROFILE)
            {
                profiles.insert(missing.clone(), profile_json(&missing, &pp));
            }
        }
        devices.push(device_json(&p, &props, &profiles));
    }
    let order = |k: &str| match k {
        "display" => 0,
        "printer" => 1,
        "scanner" => 2,
        "camera" => 3,
        "webcam" => 4,
        _ => 5,
    };
    devices.sort_by(|a, b| {
        order(a["kind"].as_str().unwrap_or(""))
            .cmp(&order(b["kind"].as_str().unwrap_or("")))
            .then_with(|| (b["embedded"] == true).cmp(&(a["embedded"] == true)))
            .then_with(|| a["model"].as_str().cmp(&b["model"].as_str()))
    });
    let sensors: Vec<Value> = paths(bus, name, "GetSensors")
        .iter()
        .filter_map(|p| {
            let s = getall(bus, name, p, SENSOR).ok()?;
            Some(json!({
                "path": p,
                "id": string(&s["SensorId"]),
                "kind": string(&s["Kind"]),
                "model": string(&s["Model"]),
                "vendor": string(&s["Vendor"]),
                "serial": string(&s["Serial"]),
                "capabilities": strings(&s["Capabilities"]),
            }))
        })
        .collect();
    json!({
        "available": true,
        "devices": devices,
        "profiles": profiles.into_values().collect::<Vec<_>>(),
        "sensors": sensors,
    })
}

/// colord's refusal: an object it does not know is "not found", no colord "unavailable", polkit's answers as
/// `call_interactive` maps them.
fn daemon_error(error: ApiError, lang: Lang) -> ApiError {
    let message = error.message.to_ascii_lowercase();
    if error.key.as_deref() == Some("core.callFailed") {
        if message.contains("not provided by any") || message.contains("serviceunknown") {
            return unavailable(lang);
        }
        if message.contains("does not exist")
            || message.contains("not found")
            || message.contains("unknown object")
        {
            return not_found(lang);
        }
    }
    error
}

fn write(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<glib::Variant>,
    lang: Lang,
) -> Result<glib::Variant, ApiError> {
    call_interactive(bus, name, path, iface, method, args.as_ref(), lang)
        .map_err(|e| daemon_error(e, lang))
}

fn op(p: &str) -> glib::variant::ObjectPath {
    object_path(p).expect("checked by path_arg")
}

/// `devices.color.setEnabled {device, on}`: "Kleurbeheer gebruiken voor dit apparaat" (Device.SetEnabled).
pub(super) fn set_enabled(
    bus: &gio::DBusConnection,
    name: &str,
    device: &str,
    on: bool,
    lang: Lang,
) -> ApiResult {
    write(
        bus,
        name,
        device,
        DEVICE,
        "SetEnabled",
        Some((on,).to_variant()),
        lang,
    )?;
    Ok(json!({"ok": true}))
}

/// `devices.color.makeDefault {device, profile}`: the profile's radio (Device.MakeProfileDefault).
pub(super) fn make_default(
    bus: &gio::DBusConnection,
    name: &str,
    device: &str,
    profile: &str,
    lang: Lang,
) -> ApiResult {
    write(
        bus,
        name,
        device,
        DEVICE,
        "MakeProfileDefault",
        Some((op(profile),).to_variant()),
        lang,
    )?;
    Ok(json!({"ok": true}))
}

/// `devices.color.addProfile {device, profile}`: "Profiel toevoegen" of a known profile (AddProfile "hard", as
/// GNOME's dialog adds a profile the user chose).
pub(super) fn add_profile(
    bus: &gio::DBusConnection,
    name: &str,
    device: &str,
    profile: &str,
    lang: Lang,
) -> ApiResult {
    write(
        bus,
        name,
        device,
        DEVICE,
        "AddProfile",
        Some(("hard", op(profile)).to_variant()),
        lang,
    )?;
    Ok(json!({"ok": true}))
}

/// `devices.color.removeProfile {device, profile}` (Device.RemoveProfile).
pub(super) fn remove_profile(
    bus: &gio::DBusConnection,
    name: &str,
    device: &str,
    profile: &str,
    lang: Lang,
) -> ApiResult {
    write(
        bus,
        name,
        device,
        DEVICE,
        "RemoveProfile",
        Some((op(profile),).to_variant()),
        lang,
    )?;
    Ok(json!({"ok": true}))
}

/// `devices.color.installSystemWide {profile, device?}`: "Instellen voor alle gebruikers" (Profile.InstallSystemWide,
/// the administrator password), then the device's default when it was named (GNOME's button does both).
pub(super) fn install_system_wide(
    bus: &gio::DBusConnection,
    name: &str,
    profile: &str,
    device: Option<&str>,
    lang: Lang,
) -> ApiResult {
    write(bus, name, profile, PROFILE, "InstallSystemWide", None, lang)?;
    if let Some(device) = device {
        make_default(bus, name, device, profile, lang)?;
    }
    Ok(json!({"ok": true}))
}

/// Whether bytes are an ICC profile: at least the 128-byte header, the "acsp" signature at 36 and the size the
/// header gives (cd_icc_load refuses the rest).
pub(super) fn is_icc(bytes: &[u8]) -> bool {
    bytes.len() >= 132
        && &bytes[36..40] == b"acsp"
        && u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize <= bytes.len()
}

/// Where an imported profile goes: `<data dir>/icc/<file name>`, the same file again when it is there with the same
/// bytes, else a free name ("name-2.icc").
pub(super) fn import_target(icc_dir: &Path, source: &Path, bytes: &[u8]) -> PathBuf {
    let file = source
        .file_name()
        .map(|f| f.to_string_lossy().replace(['/', '\0'], "_"))
        .filter(|f| !f.is_empty() && !f.starts_with('.'))
        .unwrap_or_else(|| "profile.icc".into());
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (file.clone(), String::new()),
    };
    for n in 1..1000 {
        let candidate = if n == 1 {
            icc_dir.join(&file)
        } else {
            icc_dir.join(format!("{stem}-{n}{ext}"))
        };
        match std::fs::read(&candidate) {
            Ok(existing) if existing == bytes => return candidate,
            Ok(_) => continue,
            Err(_) => return candidate,
        }
    }
    icc_dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

/// `devices.color.importProfile {device, path}`: GNOME's "Bestand importeren…" (cd_client_import_profile): the
/// ICC copied to the user's `icc` folder, found in colord (gsd's colour plugin registers that folder's files) or
/// created there (CreateProfileWithFd, scope temp, id `icc-<md5>`, as cd-icc-store.c), then added to the device.
pub(super) fn import_profile(
    bus: &gio::DBusConnection,
    name: &str,
    device: &str,
    source: &Path,
    icc_dir: &Path,
    lang: Lang,
) -> ApiResult {
    let bytes = std::fs::read(source).map_err(|_| super::bad(lang, "devices.notIcc"))?;
    if bytes.len() > 64 << 20 || !is_icc(&bytes) {
        return Err(super::bad(lang, "devices.notIcc"));
    }
    let failed = |e: &str| {
        ApiError::tr(
            "failed",
            lang,
            "devices.importFailed",
            &json!({"error": e.chars().take(200).collect::<String>()}),
        )
    };
    std::fs::create_dir_all(icc_dir).map_err(|e| failed(&e.to_string()))?;
    let target = import_target(icc_dir, source, &bytes);
    if !target.exists() {
        std::fs::write(&target, &bytes).map_err(|e| failed(&e.to_string()))?;
    }
    let filename = target.to_string_lossy().into_owned();
    let find = || {
        call(
            bus,
            name,
            MANAGER_PATH,
            MANAGER,
            "FindProfileByFilename",
            Some(&(filename.as_str(),).to_variant()),
            2000,
        )
        .ok()
        .map(|v| string(&first(&v)))
        .filter(|p| !p.is_empty())
    };
    // gsd's ICC store registers a new file in the folder within a moment
    let mut profile = None;
    for _ in 0..15 {
        profile = find();
        if profile.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let profile = match profile {
        Some(p) => p,
        None => {
            let checksum = glib::compute_checksum_for_data(glib::ChecksumType::Md5, &bytes)
                .map(|g| g.to_string())
                .unwrap_or_default();
            let file = std::fs::File::open(&target).map_err(|e| failed(&e.to_string()))?;
            let fds = gio::UnixFDList::from_array([file]);
            let mut props = BTreeMap::new();
            props.insert("Filename".to_owned(), filename.clone());
            props.insert("FILE_checksum".to_owned(), checksum.clone());
            let args = (
                format!("icc-{checksum}"),
                "temp",
                glib::variant::Handle(0),
                props,
            )
                .to_variant();
            let reply = call_with_fds(bus, name, "CreateProfileWithFd", &args, &fds)
                .map_err(|e| daemon_error(interactive_error(&e.to_string(), lang), lang))?;
            string(&first(&reply))
        }
    };
    if object_path(&profile).is_err() {
        return Err(failed("no profile"));
    }
    add_profile(bus, name, device, &profile, lang)?;
    Ok(json!({"ok": true, "profile": profile, "filename": filename}))
}

/// A call that passes file descriptors, with interactive authorization. gio 0.18's
/// `call_with_unix_fd_list_sync` asserts on a reply without descriptors (colord's CreateProfileWithFd answers none),
/// so the C function is called without an out list.
fn call_with_fds(
    bus: &gio::DBusConnection,
    name: &str,
    method: &str,
    args: &glib::Variant,
    fds: &gio::UnixFDList,
) -> Result<glib::Variant, glib::Error> {
    use glib::translate::{FromGlibPtrFull, IntoGlib, ToGlibPtr};
    let mut error = std::ptr::null_mut();
    let reply = unsafe {
        gio::ffi::g_dbus_connection_call_with_unix_fd_list_sync(
            bus.to_glib_none().0,
            name.to_glib_none().0,
            MANAGER_PATH.to_glib_none().0,
            MANAGER.to_glib_none().0,
            method.to_glib_none().0,
            args.to_glib_none().0,
            std::ptr::null(),
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION.into_glib(),
            INTERACTIVE_TIMEOUT_MS,
            fds.to_glib_none().0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut error,
        )
    };
    if error.is_null() {
        Ok(unsafe { glib::Variant::from_glib_full(reply) })
    } else {
        Err(unsafe { glib::Error::from_glib_full(error) })
    }
}

/// A sensor's colord id (the calibration's second argument, as GNOME passes cd_sensor_get_id).
fn sensor_id(bus: &gio::DBusConnection, name: &str, path: &str) -> Option<String> {
    getall(bus, name, path, SENSOR)
        .ok()
        .map(|s| string(&s["SensorId"]))
        .filter(|s| !s.is_empty())
}

/// `devices.color.calibrate.start {device, quality, kind, whitepoint, title, sensor?}`: colord-session's
/// ColorHelper.Display Start(device id, sensor id, {Quality, Whitepoint, Title, DeviceKind}), as
/// cc-color-calibrate.c; progress, samples and questions come back as `devices.calibrate` events.
#[allow(clippy::too_many_arguments)]
pub(super) fn calibrate_start(
    bus: &gio::DBusConnection,
    name: &str,
    session: &gio::DBusConnection,
    helper: &str,
    device: &str,
    sensor: Option<&str>,
    options: (u32, u32, u32, &str),
    lang: Lang,
) -> ApiResult {
    let device_id = getall(bus, name, device, DEVICE)
        .map(|d| string(&d["DeviceId"]))
        .map_err(|_| not_found(lang))?;
    let sensor_path = match sensor {
        Some(s) => s.to_owned(),
        None => paths(bus, name, "GetSensors")
            .into_iter()
            .next()
            .ok_or_else(|| {
                ApiError::tr(
                    "unavailable",
                    lang,
                    "devices.noSensor",
                    crate::i18n::NO_PARAMS,
                )
            })?,
    };
    let sensor_id = sensor_id(bus, name, &sensor_path).ok_or_else(|| {
        ApiError::tr(
            "unavailable",
            lang,
            "devices.noSensor",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let (quality, kind, whitepoint, title) = options;
    let dict = glib::VariantDict::new(None);
    dict.insert_value("Quality", &quality.to_variant());
    dict.insert_value("Whitepoint", &whitepoint.to_variant());
    dict.insert_value("Title", &title.to_variant());
    dict.insert_value("DeviceKind", &kind.to_variant());
    let args = glib::Variant::tuple_from_iter([
        device_id.to_variant(),
        sensor_id.to_variant(),
        dict.end(),
    ]);
    helper_call(session, helper, "Start", Some(args), lang)?;
    Ok(json!({"ok": true, "deviceId": device_id, "sensorId": sensor_id}))
}

/// Mutter's calibration session of one display: its object path and its CRTC's gamma LUT size.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Gamma {
    pub path: String,
    pub size: u32,
}

/// The calibration can't run here (no Mutter colour manager, no connector, no gamma LUT): GNOME's setup error.
fn no_calibration(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsupported",
        lang,
        "devices.noCalibration",
        crate::i18n::NO_PARAMS,
    )
}

/// cc-color-calibrate.c's setup: the display's connector (colord's `XRANDR_name` metadata, which Mutter sets),
/// Mutter's CalibrateMonitor for it and the session's GammaLutSize ("gamma size is zero": no calibration).
pub(super) fn gamma_session(
    bus: &gio::DBusConnection,
    name: &str,
    session: &gio::DBusConnection,
    mutter: &str,
    device: &str,
    lang: Lang,
) -> Result<Gamma, ApiError> {
    let d = getall(bus, name, device, DEVICE).map_err(|_| not_found(lang))?;
    let connector = string(&d["Metadata"]["XRANDR_name"]);
    if connector.is_empty() {
        return Err(no_calibration(lang));
    }
    let reply = call(
        session,
        mutter,
        MUTTER_PATH,
        MUTTER,
        "CalibrateMonitor",
        Some(&(connector.as_str(),).to_variant()),
        5000,
    )
    .map_err(|_| no_calibration(lang))?;
    let Some(path) = reply.child_value(0).str().map(str::to_owned) else {
        return Err(no_calibration(lang));
    };
    let size = call(
        session,
        mutter,
        &path,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(MUTTER_CALIBRATION, "GammaLutSize").to_variant()),
        3000,
    )
    .ok()
    .and_then(|v| v.child_value(0).as_variant())
    // any integer type (gnome-control-center reads it as a guint)
    .and_then(|v| crate::services::system::common::variant_json(&v).as_u64())
    .map_or(0, |n| u32::try_from(n).unwrap_or(0));
    let gamma = Gamma { path, size };
    if gamma.size == 0 {
        stop_gamma(session, mutter, &gamma);
        return Err(no_calibration(lang));
    }
    Ok(gamma)
}

/// UpdateGamma's argument (a(ddd)) as (red, green, blue) points; None when it holds none.
pub(super) fn gamma_ramp(params: &glib::Variant) -> Option<Vec<(f64, f64, f64)>> {
    let array = params.try_child_value(0)?;
    let ramp: Vec<(f64, f64, f64)> = (0..array.n_children())
        .filter_map(|i| array.child_value(i).get::<(f64, f64, f64)>())
        .collect();
    (!ramp.is_empty()).then_some(ramp)
}

/// colord-session's ramp at `size` points, as cc-color-calibrate.c's set_output_gamma: point i mixes the ramp's
/// points floor(m) and ceil(m) at m = (len - 1) / (size - 1) * i (linear), scaled to 0..0xffff (truncated).
pub(super) fn gamma_lut(ramp: &[(f64, f64, f64)], size: u32) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let (mut red, mut green, mut blue) = (Vec::new(), Vec::new(), Vec::new());
    if ramp.is_empty() {
        return (red, green, blue);
    }
    let steps = (ramp.len() - 1) as u64;
    let span = u64::from(size.max(2) - 1);
    let scale = |x: f64| (x.clamp(0.0, 1.0) * 65535.0) as u16;
    for i in 0..u64::from(size) {
        // integer floor and remainder: no libm call
        let (low, rest) = ((steps * i) / span, (steps * i) % span);
        let high = if rest > 0 { low + 1 } else { low };
        let mix = rest as f64 / span as f64;
        let (a, b) = (ramp[low as usize], ramp[high as usize]);
        red.push(scale(a.0 + (b.0 - a.0) * mix));
        green.push(scale(a.1 + (b.1 - a.1) * mix));
        blue.push(scale(a.2 + (b.2 - a.2) * mix));
    }
    (red, green, blue)
}

/// The ramp into the display's gamma LUT (the session's SetCrtcGammaLut).
pub(super) fn set_gamma(
    session: &gio::DBusConnection,
    mutter: &str,
    gamma: &Gamma,
    ramp: &[(f64, f64, f64)],
) -> Result<(), String> {
    let (red, green, blue) = gamma_lut(ramp, gamma.size);
    call(
        session,
        mutter,
        &gamma.path,
        MUTTER_CALIBRATION,
        "SetCrtcGammaLut",
        Some(&(red, green, blue).to_variant()),
        3000,
    )
    .map(drop)
}

/// The session's Stop (Mutter restores the display's own curves); errors do not matter (it may be gone).
pub(super) fn stop_gamma(session: &gio::DBusConnection, mutter: &str, gamma: &Gamma) {
    let _ = call(
        session,
        mutter,
        &gamma.path,
        MUTTER_CALIBRATION,
        "Stop",
        None,
        3000,
    );
}

/// Cancel or Resume of the running calibration.
pub(super) fn helper_call(
    session: &gio::DBusConnection,
    helper: &str,
    method: &str,
    args: Option<glib::Variant>,
    lang: Lang,
) -> ApiResult {
    call_interactive(
        session,
        helper,
        HELPER_PATH,
        HELPER_DISPLAY,
        method,
        args.as_ref(),
        lang,
    )
    .map_err(|e| {
        let message = e.message.to_ascii_lowercase();
        if e.key.as_deref() == Some("core.callFailed")
            && (message.contains("not provided by any") || message.contains("serviceunknown"))
        {
            ApiError::tr(
                "unsupported",
                lang,
                "devices.noCalibration",
                crate::i18n::NO_PARAMS,
            )
        } else {
            e
        }
    })?;
    Ok(json!({"ok": true}))
}

/// A ColorHelper.Display signal as the `devices.calibrate` event's data (None for a signal the page ignores).
pub(super) fn calibrate_event(member: &str, params: &glib::Variant) -> Option<Value> {
    let v = |i: usize| crate::services::system::common::variant_json(&params.child_value(i));
    Some(match member {
        "UpdateSample" => json!({
            "state": "sample",
            "sample": {"red": v(0), "green": v(1), "blue": v(2)},
        }),
        "InteractionRequired" => json!({
            "state": "interaction",
            "code": v(0),
            "message": v(1),
            "image": v(2),
        }),
        "Finished" => {
            let details = v(1);
            json!({
                "state": "finished",
                "error": v(0),
                "ok": v(0) == json!(0),
                "profile": details.get("ProfileId").cloned().unwrap_or(Value::Null),
                "details": details,
            })
        }
        // the ramp itself went to Mutter (Devices::apply_gamma); the page only hears that it changed
        "UpdateGamma" => json!({"state": "gamma"}),
        _ => return None,
    })
}
