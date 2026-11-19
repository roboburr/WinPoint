//! `machine.info`: what GNOME Settings' About panel and its System Details window show beyond `system.info`
//! (cc-about-page.c, cc-system-details-window.c): the device name and model and the firmware from hostname1, the
//! OS name, build and type from os-release, GNOME's version from GNOME Shell's own ShellVersion (amendment 37: there
//! is no gnome-version.xml on Ubuntu), the windowing system, systemd's Virtualization, the graphics from
//! switcheroo-control and the disk capacity from UDisks2. Every part falls back to the files the daemons read, or is
//! left out ("" / null), as GNOME hides a row it has no value for.
use super::{Buses, HOSTNAME, SHELL, SWITCHEROO, SYSTEMD, UDISKS};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// `KEY=value` lines (os-release, machine-info), the values unquoted as a shell would.
pub(crate) fn key_values(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = if v.len() >= 2
                && ((v.starts_with('"') && v.ends_with('"'))
                    || (v.starts_with('\'') && v.ends_with('\'')))
            {
                v[1..v.len() - 1]
                    .replace("\\\"", "\"")
                    .replace("\\$", "$")
                    .replace("\\\\", "\\")
            } else {
                v.to_owned()
            };
            Some((k.trim().to_owned(), v))
        })
        .collect()
}

/// GNOME's names of systemd's virtualization ids (cc-system-details-window.c get_virtualization_label); "" for none.
pub(crate) fn virtualization_label(id: &str) -> String {
    let table = [
        ("kvm", "KVM"),
        ("qemu", "QEmu"),
        ("vmware", "VMware"),
        ("microsoft", "Microsoft"),
        ("oracle", "Oracle"),
        ("xen", "Xen"),
        ("bochs", "Bochs"),
        ("chroot", "chroot"),
        ("openvz", "openvz"),
        ("lxc", "lxc"),
        ("lxc-libvirt", "lxc (libvirt)"),
        ("systemd-nspawn", "systemd (nspawn)"),
    ];
    match id {
        "" | "none" => String::new(),
        id => table
            .iter()
            .find(|(k, _)| *k == id)
            .map_or(id.to_owned(), |(_, v)| (*v).to_owned()),
    }
}

/// "Wayland" or "X11" (GNOME: the GDK backend; here the session's type), "" when unknown.
pub(crate) fn windowing_system(session_type: Option<&str>, wayland: bool) -> &'static str {
    match session_type {
        Some("wayland") => "Wayland",
        Some("x11") => "X11",
        _ if wayland => "Wayland",
        _ => "",
    }
}

/// The disk capacity GNOME shows: the sum of every drive's Size that is neither removable nor ejectable
/// (get_primary_disk_info), from UDisks2's GetManagedObjects as JSON.
pub(crate) fn disk_capacity(objects: &Value) -> Option<u64> {
    let total: u64 = disk_drives(objects)
        .iter()
        .filter_map(|d| d["size"].as_u64())
        .sum();
    (total > 0).then_some(total)
}

/// The drives that capacity sums, `{model, size}` each (UDisks2's Vendor and Model, its Size), so Info lists the
/// drives of its total and no others.
pub(crate) fn disk_drives(objects: &Value) -> Vec<Value> {
    let Some(map) = objects.as_object() else {
        return Vec::new();
    };
    map.values()
        .filter_map(|ifaces| ifaces.get("org.freedesktop.UDisks2.Drive"))
        .filter(|d| d["Removable"] != true && d["Ejectable"] != true)
        .filter_map(|d| {
            let size = d["Size"].as_u64().filter(|s| *s > 0)?;
            let vendor = d["Vendor"].as_str().unwrap_or("").trim();
            let model = d["Model"].as_str().unwrap_or("").trim();
            let name = if vendor.is_empty() || model.starts_with(vendor) {
                model.to_owned()
            } else {
                format!("{vendor} {model}").trim().to_owned()
            };
            Some(json!({"model": name, "size": size}))
        })
        .collect()
}

/// The graphics of switcheroo-control's GPUs property (aa{sv} as JSON): the default GPU first.
pub(crate) fn gpu_names(gpus: &Value) -> Vec<String> {
    let mut list: Vec<(bool, String)> = gpus
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let name = g["Name"].as_str()?.trim();
            (!name.is_empty()).then(|| (g["Default"] != true, name.to_owned()))
        })
        .collect();
    list.sort_by_key(|(not_default, _)| *not_default);
    list.into_iter().map(|(_, n)| n).collect()
}

fn read(b: &Buses, path: &str) -> String {
    std::fs::read_to_string(b.file(path))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// The model GNOME shows (get_hardware_model_string): vendor and model, the model alone when the vendor is empty.
pub(crate) fn model_text(vendor: &str, model: &str) -> String {
    match (vendor.trim(), model.trim()) {
        ("", m) => m.to_owned(),
        (v, "") => v.to_owned(),
        (v, m) => format!("{v} {m}"),
    }
}

/// `machine.info {}`.
pub(crate) fn info(b: &Buses) -> Value {
    let (_, hpath, hiface) = HOSTNAME;
    let h = b
        .hostname()
        .and_then(|p| p.props(hpath, hiface).ok())
        .unwrap_or(Value::Null);
    let text = |k: &str| h[k].as_str().unwrap_or("").trim().to_owned();
    let machine_info = key_values(&read(b, "/etc/machine-info"));
    let uname = nix_uname();
    let hostname = Some(text("Hostname"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uname.0.clone());
    let static_name = Some(text("StaticHostname"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| read(b, "/etc/hostname"));
    let pretty = Some(text("PrettyHostname"))
        .filter(|s| !s.is_empty())
        .or_else(|| machine_info.get("PRETTY_HOSTNAME").cloned())
        .unwrap_or_default();
    let dmi = |f: &str| read(b, &format!("/sys/class/dmi/id/{f}"));
    let (vendor, model) = if h.is_object() {
        (text("HardwareVendor"), text("HardwareModel"))
    } else {
        (dmi("sys_vendor"), dmi("product_name"))
    };
    let firmware = Some(text("FirmwareVersion"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dmi("bios_version"));
    let os = key_values(&{
        let s = read(b, "/etc/os-release");
        if s.is_empty() {
            read(b, "/usr/lib/os-release")
        } else {
            s
        }
    });
    let os_name = os
        .get("PRETTY_NAME")
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| Some(text("OperatingSystemPrettyName")).filter(|s| !s.is_empty()))
        .unwrap_or_default();
    // GNOME Shell's own version (amendment 37)
    let (_, spath, siface) = SHELL;
    let gnome = b
        .shell()
        .and_then(|p| p.prop(spath, siface, "ShellVersion").ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let (_, vpath, viface) = SYSTEMD;
    let virt = b
        .systemd()
        .and_then(|p| p.prop(vpath, viface, "Virtualization").ok())
        .and_then(|v| v.as_str().map(virtualization_label))
        .unwrap_or_default();
    let (_, gpath, giface) = SWITCHEROO;
    let gpus = b
        .switcheroo()
        .and_then(|p| p.prop(gpath, giface, "GPUs").ok())
        .map(|v| gpu_names(&v))
        .unwrap_or_default();
    let objects = b
        .udisks()
        .and_then(|p| {
            p.call(
                UDISKS.1,
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                None,
            )
            .ok()
        })
        .map(|v| crate::services::system::common::variant_json(&v.child_value(0)))
        .unwrap_or(Value::Null);
    let disks = disk_capacity(&objects);
    let drives = disk_drives(&objects);
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    json!({
        "hostname": hostname,
        "staticHostname": static_name,
        "prettyHostname": pretty,
        "chassis": text("Chassis"),
        "hardwareVendor": vendor,
        "hardwareModel": model,
        "model": model_text(&vendor, &model),
        "firmwareVersion": firmware,
        "osName": os_name,
        "osShortName": os.get("NAME").cloned().unwrap_or_default(),
        "osVersion": os.get("VERSION").or_else(|| os.get("VERSION_ID")).cloned().unwrap_or_default(),
        "osBuild": os.get("BUILD_ID").cloned().unwrap_or_default(),
        "osBits": usize::BITS,
        "architecture": uname.1,
        "gnomeVersion": gnome,
        "windowingSystem": windowing_system(session.as_deref(), std::env::var_os("WAYLAND_DISPLAY").is_some()),
        "virtualization": virt,
        "kernel": format!("{} {}", uname.2, uname.3).trim().to_owned(),
        "gpus": gpus,
        "diskCapacity": disks,
        "disks": drives,
        "canRename": h.is_object(),
    })
}

/// (nodename, machine, sysname, release) of uname(2).
fn nix_uname() -> (String, String, String, String) {
    let u = unsafe {
        let mut u = std::mem::zeroed::<libc::utsname>();
        libc::uname(&mut u);
        u
    };
    let s = |p: &[libc::c_char]| unsafe {
        std::ffi::CStr::from_ptr(p.as_ptr())
            .to_string_lossy()
            .into_owned()
    };
    (s(&u.nodename), s(&u.machine), s(&u.sysname), s(&u.release))
}
