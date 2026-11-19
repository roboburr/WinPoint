//! Instellingen › Systeem › Info (6ebf169f:vendor/ui/js/apps/settings/pages/about.js; settings parity batch E,
//! notes/claude/settings-parity.md §2): Windows 11 25H2's About page with GNOME Settings' About and System Details
//! behind it: the device's name and model with "Naam van deze pc wijzigen" (machine.setName through hostname1), the
//! four cards (Processor, Geïnstalleerd RAM, Opslag, Grafische kaart), Apparaatspecificaties (with GNOME's model,
//! graphics, disk capacity and firmware) and the OS's specifications (GNOME's OS name, build, GNOME version from
//! GNOME Shell's ShellVersion, windowing system, virtualisation, kernel; WinPoint under Windows' "Ervaring"), each with
//! "Kopiëren", and the related links.
use super::system::rename_hint;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::preview::DeviceArt;
use crate::widgets::Wrap;
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("about", about)];
pub const CSS: &str = "";

type Rows = Vec<(String, Option<String>)>;

/// Opslag: GNOME's disk capacity (UDisks2: every drive that is neither removable nor ejectable) and the drives it
/// sums (machine.info `disks`), so the list adds up to the total in front of it; without UDisks the sum of
/// system.info's disks that are not on USB, and those disks.
pub(crate) fn storage_drives(machine: &Value, info: &Value) -> (Option<f64>, Vec<Value>) {
    if let Some(total) = machine["diskCapacity"].as_f64().filter(|v| *v > 0.0) {
        // the drives that total sums (a host without the list: the total alone, no list that would not add up)
        return (Some(total), machine["disks"].as_array().cloned().unwrap_or_default());
    }
    let internal: Vec<Value> = info["disks"].as_array().into_iter().flatten().filter(|d| d["kind"] != "usb").cloned().collect();
    let total: f64 = internal.iter().filter_map(|d| d["size"].as_f64()).sum();
    ((total > 0.0).then_some(total), internal)
}

/// The specifications as a two-column list (`.st-specs`), selectable.
fn spec_list(rows: &Rows) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.add_css_class("st-specs");
    grid.set_column_spacing(24);
    grid.set_row_spacing(10);
    let mut r = 0;
    for (k, v) in rows {
        let Some(v) = v else { continue };
        let key = c::label(k, "st-spec-key", true);
        key.set_size_request(130, -1);
        key.set_max_width_chars(28);
        key.set_valign(gtk::Align::Start);
        let val = c::label(v, "st-spec-value", true);
        val.set_selectable(true);
        val.set_hexpand(true);
        val.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        grid.attach(&key, 0, r, 1, 1);
        grid.attach(&val, 1, r, 1, 1);
        r += 1;
    }
    // the expander's nested padding goes on this box, the list's own 4 px on the grid
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&grid);
    wrap.upcast()
}

fn copy_button(p: &Page, rows: Rows, title: &str) -> gtk::Button {
    let b = c::button(t("Kopiëren", "Copy"), c::Variant::Standard);
    let (pp, title) = (p.clone(), title.to_owned());
    b.connect_clicked(move |_| {
        let mut text = format!("{title}\n");
        text.push_str(&rows.iter().filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}\t{v}"))).collect::<Vec<_>>().join("\n"));
        let ok = common::copy_text(&text);
        pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2200));
    });
    b
}

fn some(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn mode_name(m: &str) -> String {
    match m {
        "layer" => t("bureaublad", "desktop").into(),
        "preview" => t("voorbeeldvenster", "preview window").into(),
        "serve" => t("browser", "browser").into(),
        "app" => t("los venster", "separate window").into(),
        other => other.into(),
    }
}

/// Windows' "Systeemtype": "64-bits besturingssysteem, x64-processor" from the OS's width and the machine's
/// architecture (uname).
pub fn system_type(bits: u64, architecture: &str) -> String {
    let arch = match architecture {
        "x86_64" | "amd64" => "x64",
        "aarch64" | "arm64" => "ARM64",
        "i386" | "i486" | "i586" | "i686" => "x86",
        "" => return String::new(),
        other => other,
    };
    common::fill(
        t("{bits}-bits besturingssysteem, {arch}-processor", "{bits}-bit operating system, {arch}-based processor"),
        &[("bits", &bits.to_string()), ("arch", arch)],
    )
}

/// "Geïnstalleerd RAM": the installed memory, and the usable memory next to it when they differ (Windows: "16,0 GB
/// (15,7 GB bruikbaar)").
pub fn ram_text(installed: Option<f64>, usable: Option<f64>) -> Option<String> {
    match (installed.filter(|v| *v > 0.0), usable.filter(|v| *v > 0.0)) {
        (Some(i), Some(u)) if (i - u).abs() > 1024.0 * 1024.0 => Some(common::fill(
            t("{total} ({usable} bruikbaar)", "{total} ({usable} usable)"),
            &[("total", &kit::format::bytes(i)), ("usable", &kit::format::bytes(u))],
        )),
        (Some(v), _) | (None, Some(v)) => Some(kit::format::bytes(v)),
        (None, None) => None,
    }
}

fn about(p: Page) -> PageFuture {
    Box::pin(async move {
        let (info, machine, upd, sh) = (p.get("system.info", json!({})), p.get("machine.info", json!({})), p.get("system.updates", json!({})), p.get("shell.info", json!({})));
        let (info, machine, upd, sh) = (info.await, machine.await.unwrap_or(Value::Null), upd.await, sh.await);
        if !p.alive() {
            return Content::new();
        }
        let sh = sh.unwrap_or_else(common::shell_info);
        let mut out: Content = Vec::new();
        if info.is_none() {
            out.push(common::notice(t("Niet alle informatie is beschikbaar op deze computer.", "Not all information is available on this computer.")).widget().clone().upcast());
        }
        let info = info.unwrap_or(Value::Null);
        let name = some(&machine["prettyHostname"]).or_else(|| super::system::pc_name(&info));
        let model = some(&machine["model"]).or_else(|| some(&info["deviceModel"]));

        // ---- head: the monitor, the name, Naam van deze pc wijzigen
        let art = DeviceArt::new(true);
        {
            let art = art.clone();
            p.on_settings(move |s, _| art.update(s));
        }
        let head = Wrap::new(20, 12);
        head.add_css_class("st-about-head");
        head.append(&art);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.set_size_request(200, -1);
        let name_label = c::label(name.as_deref().unwrap_or("–"), "st-about-name", true);
        name_label.set_widget_name("st-about-name");
        text.append(&name_label);
        text.append(&c::label(model.as_deref().unwrap_or(""), "st-about-model", true));
        head.append(&text);
        let rename = c::button(t("Naam van deze pc wijzigen", "Rename this PC"), c::Variant::Standard);
        rename.set_valign(gtk::Align::Center);
        rename.set_widget_name("about-rename");
        rename.set_sensitive(machine["canRename"] != false);
        let (pp, current) = (p.clone(), name.clone().unwrap_or_default());
        rename.connect_clicked(move |_| rename_hint(&pp, &current));
        head.append(&rename);
        out.push(head.upcast());

        // ---- the rows
        let cpu = &info["cpu"];
        let cpu_text = some(&cpu["model"]).map(|model| match cpu["cores"].as_i64().filter(|n| *n > 0) {
            Some(cores) => {
                let threads = cpu["threads"].as_i64().filter(|n| *n > 0).unwrap_or(cores);
                let c = common::plural(cores, t("{n} kern", "{n} core"), t("{n} kernen", "{n} cores"));
                let th = common::plural(threads, t("{n} thread", "{n} thread"), t("{n} threads", "{n} threads"));
                format!("{model} ({})", common::fill(t("{cores}, {threads}", "{cores}, {threads}"), &[("cores", &c), ("threads", &th)]))
            }
            None => model,
        });
        let join = |v: &Value, f: &dyn Fn(&Value) -> String| -> Option<String> {
            let s = v.as_array().map(|a| a.iter().map(f).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            if s.is_empty() { None } else { Some(s) }
        };
        let gpus = join(&machine["gpus"], &|g: &Value| g.as_str().unwrap_or("").to_owned()).or_else(|| join(&info["gpus"], &|g: &Value| g.as_str().unwrap_or("").to_owned()));
        let (capacity, drive_list) = storage_drives(&machine, &info);
        let drives = join(&Value::Array(drive_list), &|d: &Value| format!("{} ({})", d["model"].as_str().unwrap_or(""), kit::format::bytes(d["size"].as_f64().unwrap_or(0.0))).trim().to_owned());
        let storage_total = capacity.map(kit::format::bytes);
        let storage = match (&storage_total, &drives) {
            (Some(total), Some(list)) => Some(format!("{total}: {list}")),
            (total, list) => total.clone().or_else(|| list.clone()),
        };
        let ram = ram_text(info["memory"]["installed"].as_f64(), info["memory"]["total"].as_f64());
        let arch = machine["architecture"].as_str().or(info["architecture"].as_str()).unwrap_or("");
        let dev_rows: Rows = vec![
            (t("Apparaatnaam", "Device name").into(), name.clone()),
            (t("Model", "Model").into(), model.clone()),
            (t("Processor", "Processor").into(), cpu_text.clone()),
            (t("Geïnstalleerd RAM", "Installed RAM").into(), ram.clone()),
            (t("Grafische kaart", "Graphics card").into(), gpus.clone()),
            (t("Opslag", "Storage").into(), storage.clone()),
            (t("Firmwareversie", "Firmware version").into(), some(&machine["firmwareVersion"])),
            (t("Systeemtype", "System type").into(), Some(system_type(machine["osBits"].as_u64().unwrap_or(64), arch)).filter(|s| !s.is_empty())),
        ];

        // Windows 25H2: the four cards at the top
        let tiles = Wrap::new(8, 8);
        for (icon, title, value) in [
            ("cpu", t("Processor", "Processor"), cpu_text),
            ("memory", t("Geïnstalleerd RAM", "Installed RAM"), ram),
            ("storage", t("Opslag", "Storage"), storage_total),
            ("gpu", t("Grafische kaart", "Graphics card"), gpus),
        ] {
            if let Some(v) = value {
                tiles.append(&common::hero_tile(icon, title, &[v], None));
            }
        }
        out.push(tiles.upcast());

        let shell = some(&sh["version"]).map(|v| {
            let mut s = format!("WinPoint {v}");
            if let Some(m) = some(&sh["mode"]) {
                s.push_str(&format!(" · {}", mode_name(&m)));
            }
            if sh["dryRun"] == true {
                s.push_str(&format!(" · {}", t("testmodus", "test mode")));
            }
            s
        });
        let edition = some(&machine["osName"]).unwrap_or_else(|| common::os_label(common::s(&info["os"]), common::s(&info["osVersion"])));
        let installed = info["installedAt"].as_f64().map(|ms| kit::format::date(ms, kit::format::DateFormat::Short));
        let os_rows: Rows = vec![
            (t("Editie", "Edition").into(), (!edition.is_empty()).then_some(edition)),
            (t("Versie", "Version").into(), some(&machine["osVersion"]).or_else(|| some(&info["osVersion"]))),
            (t("Geïnstalleerd op", "Installed on").into(), installed),
            (t("Build van besturingssysteem", "OS build").into(), some(&machine["osBuild"])),
            (t("GNOME-versie", "GNOME version").into(), some(&machine["gnomeVersion"])),
            (t("Venstersysteem", "Windowing system").into(), some(&machine["windowingSystem"])),
            (t("Virtualisatie", "Virtualisation").into(), some(&machine["virtualization"])),
            (t("Kernel", "Kernel").into(), some(&machine["kernel"]).or_else(|| some(&info["kernel"]))),
            (t("Ervaring", "Experience").into(), shell),
        ];

        let device = SettingsExpander::new("pc", t("Apparaatspecificaties", "Device specifications"), "");
        device.header().set_control(&copy_button(&p, dev_rows.clone(), t("Apparaatspecificaties", "Device specifications")));
        device.header_button().set_widget_name("st-about-device");
        device.add(&spec_list(&dev_rows));
        device.set_expanded(true);
        let os_title = match some(&machine["osShortName"]) {
            Some(os) => common::fill(t("{os}-specificaties", "{os} specifications"), &[("os", &os)]),
            None => t("Besturingssysteem", "Operating system").to_owned(),
        };
        let os = SettingsExpander::new("info", &os_title, "");
        os.header().set_control(&copy_button(&p, os_rows.clone(), &os_title));
        os.header_button().set_widget_name("st-about-os");
        os.add(&spec_list(&os_rows));
        os.set_expanded(true);
        let g = SettingsGroup::new("", "");
        g.add(device.widget());
        g.add(os.widget());
        out.push(g.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "storage").widget());
        let perf = SettingsCard::clickable("chart", t("Prestaties bekijken", "View performance"), t("Processor, geheugen en schijven live in Taakbeheer", "Processor, memory and disks live in Task Manager"), c::Chevron::External);
        if let Some(b) = perf.button() {
            b.set_widget_name("about-performance");
        }
        let pp = p.clone();
        perf.connect_clicked(move || {
            let fut = pp.run_with("shell.openApp", json!({ "app": "taskmgr", "params": { "page": "performance" } }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
        related.add(perf.widget());
        if upd.is_some() {
            related.add(common::link_card(&p, "update").widget());
        }
        out.push(related.widget().clone().upcast());
        // a new name (machine.changed {what: name}): the page is built again with it
        let pp = p.clone();
        p.on("machine.changed", move |d| {
            if d["what"] == "name" {
                pp.refresh();
            }
        });
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_storage_list_adds_up_to_its_total() {
        let tb = 1e12;
        let machine = json!({"diskCapacity": 25.0 * tb, "disks": [{"model": "ST24000DM001", "size": 24.0 * tb}, {"model": "Lexar NM790", "size": 1.0 * tb}]});
        let info = json!({"disks": [{"model": "ST24000DM001", "size": 24.0 * tb, "kind": "hdd"}, {"model": "HGST HTS545050A7E680", "size": 0.5 * tb, "kind": "hdd"},
                                    {"model": "Kingston DataTraveler", "size": 0.03 * tb, "kind": "usb"}]});
        let (total, list) = storage_drives(&machine, &info);
        assert_eq!(total, Some(25.0 * tb));
        let sum: f64 = list.iter().filter_map(|d| d["size"].as_f64()).sum();
        assert_eq!(sum, 25.0 * tb, "the drives GNOME's capacity sums, not the hot-swap drive UDisks counts as removable");
        // without UDisks: system.info's disks without the USB ones, for the total and the list alike
        let (total, list) = storage_drives(&json!({}), &info);
        assert_eq!(total, Some(24.5 * tb));
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|d| d["kind"] != "usb"));
        // a capacity without a list: no list that would not add up
        assert_eq!(storage_drives(&json!({"diskCapacity": 3.0 * tb}), &info), (Some(3.0 * tb), Vec::new()));
    }

    #[test]
    fn windows_system_type_and_ram_lines() {
        assert_eq!(system_type(64, "x86_64"), t("64-bits besturingssysteem, x64-processor", "64-bit operating system, x64-based processor"));
        assert!(system_type(64, "aarch64").contains("ARM64"));
        assert_eq!(system_type(64, ""), "");
        let gib = 1024.0 * 1024.0 * 1024.0;
        assert!(ram_text(Some(16.0 * gib), Some(15.6 * gib)).unwrap().contains('('), "installed and usable");
        assert!(!ram_text(Some(16.0 * gib), Some(16.0 * gib)).unwrap().contains('('));
        assert!(ram_text(None, Some(8.0 * gib)).is_some());
        assert_eq!(ram_text(None, Some(0.0)), None);
    }
}
