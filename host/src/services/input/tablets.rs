//! The drawing tablets, pen displays and touchscreens of `input.tablets` (notes/claude/settings-parity.md §5.2,
//! amendments 8 and 9), found as GNOME Settings' Wacom panel finds them:
//!
//! * the input event devices udev's database (`/run/udev/data/c13:<minor>`) marks `ID_INPUT_TABLET` (a pen),
//!   `ID_INPUT_TABLET_PAD` (its buttons, rings, strips and dials) or `ID_INPUT_TOUCHSCREEN`, with their vendor and
//!   product id from /sys (`/sys/dev/char/13:<minor>/device/id/`). Mutter keys a tablet's settings by those ids,
//!   `/org/gnome/desktop/peripherals/tablets/<vid>:<pid>/` (`%.4x:%.4x`), so one entry per id;
//! * what libwacom knows of the model (`/usr/share/libwacom/*.tablet`, `/etc/libwacom` first): its name, whether it
//!   is built into a display (`IntegratedIn=Display`) or the computer (`System`), whether it can be turned round
//!   (`Reversible`, GNOME's "Button Location") and its pad's buttons, rings, strips and dials;
//! * the monitors from Mutter's DisplayConfig `GetCurrentState`, for the `output` key ([vendor, product, serial]).
//!
//! Nothing here writes. A stylus' settings live at `/org/gnome/desktop/peripherals/stylus/<serial>/`, or
//! `stylus/default-<vid>:<pid>/` for a pen without a serial (libmutter-18); the serials come from GTK on the page
//! (GdkSeat's tools), so a tablet's `styli` holds the serial-less id only.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `[0-9a-f]{4}:[0-9a-f]{4}`, the `{vidpid}` hole of the allow tables.
pub(crate) fn vidpid_ok(s: &str) -> bool {
    crate::services::gsettings::allow::hole_ok("vidpid", s)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Four hex digits in lower case ("056A\n" → "056a").
fn hex4(text: &str) -> Option<String> {
    let t = text.trim().to_ascii_lowercase();
    (t.len() == 4 && t.chars().all(|c| c.is_ascii_hexdigit())).then_some(t)
}

/// One input event device of a tablet, pad or touchscreen.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Device {
    pub vidpid: String,
    pub name: String,
    /// /sys's bus type: 0003 USB, 0005 Bluetooth, 0018 I²C.
    pub bus: String,
    pub pen: bool,
    pub pad: bool,
    pub touchscreen: bool,
}

/// The tablet, pad and touchscreen event devices below `root`, in udev's order of minors.
pub(crate) fn devices(root: &Path) -> Vec<Device> {
    let data = root.join("run/udev/data");
    let Ok(entries) = std::fs::read_dir(&data) else {
        return Vec::new();
    };
    let mut names: Vec<(u32, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let minor = name.strip_prefix("c13:")?.parse().ok()?;
            Some((minor, name))
        })
        .collect();
    names.sort();
    let mut out = Vec::new();
    for (minor, name) in names {
        let text = read(&data.join(&name));
        let prop = |p: &str| {
            text.lines()
                .any(|l| l.strip_prefix("E:") == Some(format!("{p}=1").as_str()))
        };
        if !prop("ID_INPUT") {
            continue;
        }
        // udev's input_id marks a pad ID_INPUT_TABLET too
        let pad = prop("ID_INPUT_TABLET_PAD");
        let pen = prop("ID_INPUT_TABLET") && !pad;
        let touchscreen = prop("ID_INPUT_TOUCHSCREEN");
        if !(pen || pad || touchscreen) {
            continue;
        }
        let sys = root.join(format!("sys/dev/char/13:{minor}"));
        // the evdev node only (/dev/input/eventN), not the legacy mouseN of the same device
        let evdev = read(&sys.join("uevent")).lines().any(|l| {
            l.strip_prefix("DEVNAME=")
                .is_some_and(|d| d.starts_with("input/event"))
        });
        if !evdev {
            continue;
        }
        let dev = sys.join("device");
        let (Some(vendor), Some(product)) = (
            hex4(&read(&dev.join("id/vendor"))),
            hex4(&read(&dev.join("id/product"))),
        ) else {
            continue;
        };
        out.push(Device {
            vidpid: format!("{vendor}:{product}"),
            name: read(&dev.join("name")).trim().to_owned(),
            bus: read(&dev.join("id/bustype")).trim().to_ascii_lowercase(),
            pen,
            pad,
            touchscreen,
        });
    }
    out
}

/// What a libwacom `.tablet` file says about a model.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Wacom {
    pub name: String,
    /// Every `DeviceMatch` entry: (bus, vid:pid, kernel device name or "").
    pub matches: Vec<(String, String, String)>,
    pub display: bool,
    pub system: bool,
    pub buttons: u32,
    pub rings: u32,
    pub strips: u32,
    pub dials: u32,
    pub reversible: Option<bool>,
}

/// One libwacom `.tablet` file (a GKeyFile: [Device], [Features], [Buttons]); None when it matches no device.
pub(crate) fn parse_wacom(text: &str) -> Option<Wacom> {
    let mut w = Wacom::default();
    let mut section = String::new();
    let mut letters: BTreeSet<String> = BTreeSet::new();
    let (mut ring_flags, mut counted) = (0u32, 0u32);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_owned();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let number = || v.parse::<u32>().unwrap_or(0);
        match (section.as_str(), k) {
            ("Device", "Name") => w.name = v.to_owned(),
            ("Device", "DeviceMatch") => {
                for m in v.split(';').filter(|m| !m.trim().is_empty()) {
                    let parts: Vec<&str> = m.trim().split('|').collect();
                    if parts.len() < 3 {
                        continue; // "generic"
                    }
                    if let (Some(vid), Some(pid)) = (hex4(parts[1]), hex4(parts[2])) {
                        let name = parts.get(3).copied().unwrap_or("").to_owned();
                        w.matches
                            .push((parts[0].to_owned(), format!("{vid}:{pid}"), name));
                    }
                }
            }
            ("Device", "IntegratedIn") => {
                for part in v.split(';') {
                    match part.trim() {
                        "Display" => w.display = true,
                        "System" => w.system = true,
                        _ => {}
                    }
                }
            }
            ("Features", "NumRings") => w.rings = number(),
            // libwacom 1.x: Ring=true, Ring2=true, Buttons=N
            ("Features", "Ring") if v == "true" => ring_flags |= 1,
            ("Features", "Ring2") if v == "true" => ring_flags |= 2,
            ("Features", "Buttons") => counted = number(),
            ("Features", "NumStrips") => w.strips = number(),
            ("Features", "NumDials") => w.dials = number(),
            ("Features", "Reversible") => w.reversible = Some(v == "true"),
            ("Buttons", "Left" | "Right" | "Top" | "Bottom") => letters.extend(
                v.split(';')
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    if w.rings == 0 {
        w.rings = ring_flags.count_ones();
    }
    w.buttons = if letters.is_empty() {
        counted
    } else {
        letters.len() as u32
    };
    (!w.matches.is_empty()).then_some(w)
}

/// Every libwacom model below `root`: /usr/share/libwacom, a file of the same name in /etc/libwacom replacing it.
pub(crate) fn wacom_db(root: &Path) -> Vec<Wacom> {
    let mut by_file: BTreeMap<String, Wacom> = BTreeMap::new();
    for dir in ["usr/share/libwacom", "etc/libwacom"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            if !file.ends_with(".tablet") {
                continue;
            }
            if let Some(w) = parse_wacom(&read(&entry.path())) {
                by_file.insert(file, w);
            }
        }
    }
    by_file.into_values().collect()
}

/// The model of a device: an entry with its kernel name first (Huion's "Tablet Monitor Pen"), then one without.
pub(crate) fn lookup<'a>(db: &'a [Wacom], vidpid: &str, name: &str) -> Option<&'a Wacom> {
    let hits: Vec<&Wacom> = db
        .iter()
        .filter(|w| w.matches.iter().any(|(_, v, _)| v == vidpid))
        .collect();
    let named = hits.iter().find(|w| {
        w.matches
            .iter()
            .any(|(_, v, n)| v == vidpid && !n.is_empty() && name.contains(n.as_str()))
    });
    let plain = hits.iter().find(|w| {
        w.matches
            .iter()
            .any(|(_, v, n)| v == vidpid && n.is_empty())
    });
    named.or(plain).or(hits.first()).copied()
}

/// A device's kernel name without the part that names the interface ("Wacom Intuos Pro M Pen" → "Wacom Intuos
/// Pro M").
fn model_name(name: &str) -> String {
    for suffix in [
        " Pen (0)", " Pen", " Stylus", " stylus", " Pad", " pad", " Finger",
    ] {
        if let Some(n) = name.strip_suffix(suffix) {
            return n.trim().to_owned();
        }
    }
    name.trim().to_owned()
}

/// `{tablets, touchscreens}` below `root` (module docs). A pad without a pen (the ExpressKey Remote) is a tablet
/// with `pen: false`: only its buttons can be mapped.
pub(crate) fn scan(root: &Path) -> Value {
    let found = devices(root);
    let mut ids: Vec<String> = Vec::new();
    for d in &found {
        if !ids.contains(&d.vidpid) {
            ids.push(d.vidpid.clone());
        }
    }
    let db = if found.iter().any(|d| d.pen || d.pad) {
        wacom_db(root)
    } else {
        Vec::new()
    };
    let mut tablets = Vec::new();
    let mut touchscreens = Vec::new();
    for id in &ids {
        let of_id: Vec<&Device> = found.iter().filter(|d| &d.vidpid == id).collect();
        let pen = of_id.iter().find(|d| d.pen);
        let pad = of_id.iter().find(|d| d.pad);
        if let Some(first) = pen.or(pad) {
            let model = lookup(&db, id, &first.name);
            let kind = match model {
                Some(w) if w.system => "builtin",
                Some(w) if w.display => "display",
                Some(_) => "tablet",
                None if first.bus == "0018" => "builtin",
                None => "tablet",
            };
            let name = model
                .map(|w| w.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| model_name(&first.name));
            let count = |f: fn(&Wacom) -> u32| model.map(f).unwrap_or(0);
            tablets.push(json!({
                "vidpid": id,
                "name": name,
                "kind": kind,
                "pen": pen.is_some(),
                "pad": pad.is_some(),
                "buttons": count(|w| w.buttons),
                "rings": count(|w| w.rings),
                "strips": count(|w| w.strips),
                "dials": count(|w| w.dials),
                "reversible": model.and_then(|w| w.reversible),
                "styli": if pen.is_some() { json!([format!("default-{id}")]) } else { json!([]) },
            }));
        }
        if let Some(touch) = of_id.iter().find(|d| d.touchscreen) {
            touchscreens.push(json!({ "vidpid": id, "name": model_name(&touch.name) }));
        }
    }
    json!({ "tablets": tablets, "touchscreens": touchscreens })
}

/// DisplayConfig's `GetCurrentState` reply.
pub(crate) const STATE_TYPE: &str = "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})";

/// The value of `key` in an `a{sv}` dictionary.
fn prop(dict: &glib::Variant, key: &str) -> Option<glib::Variant> {
    (0..dict.n_children())
        .map(|i| dict.child_value(i))
        .find(|entry| entry.child_value(0).str() == Some(key))
        .and_then(|entry| entry.child_value(1).as_variant())
}

/// The monitors of a `GetCurrentState` reply: `{connector, vendor, product, serial, name, builtin}`.
pub(crate) fn parse_monitors(reply: &glib::Variant) -> Vec<Value> {
    let Ok(ty) = glib::VariantTy::new(STATE_TYPE) else {
        return Vec::new();
    };
    if !reply.is_type(ty) {
        return Vec::new();
    }
    let list = reply.child_value(1);
    (0..list.n_children())
        .map(|i| {
            let monitor = list.child_value(i);
            let spec = monitor.child_value(0);
            let text = |j: usize| spec.child_value(j).str().unwrap_or("").to_owned();
            let props = monitor.child_value(2);
            let name = prop(&props, "display-name")
                .and_then(|v| v.str().map(str::to_owned))
                .unwrap_or_default();
            let builtin = prop(&props, "is-builtin")
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            json!({
                "connector": text(0),
                "vendor": text(1),
                "product": text(2),
                "serial": text(3),
                "name": name,
                "builtin": builtin,
            })
        })
        .collect()
}

/// The monitors from `destination`'s DisplayConfig on `bus` (tests: a fake's unique name); none on any error.
pub(crate) fn monitors_on(bus: &gio::DBusConnection, destination: &str) -> Vec<Value> {
    let Ok(ty) = glib::VariantTy::new(STATE_TYPE) else {
        return Vec::new();
    };
    bus.call_sync(
        Some(destination),
        "/org/gnome/Mutter/DisplayConfig",
        "org.gnome.Mutter.DisplayConfig",
        "GetCurrentState",
        None,
        Some(ty),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .map(|reply| parse_monitors(&reply))
    .unwrap_or_default()
}

/// The session's monitors (Mutter's DisplayConfig); none outside GNOME.
pub(crate) fn monitors() -> Vec<Value> {
    match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(bus) => monitors_on(&bus, "org.gnome.Mutter.DisplayConfig"),
        Err(_) => Vec::new(),
    }
}
