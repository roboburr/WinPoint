//! Instellingen › Systeem › Info (vendor/ui/js/apps/settings/pages/about.js): the device and the operating
//! system with "Kopiëren", and related links.
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

fn about(p: Page) -> PageFuture {
    Box::pin(async move {
        let (info, upd, sh) = (p.get("system.info", json!({})), p.get("system.updates", json!({})), p.get("shell.info", json!({})));
        let (info, upd, sh) = (info.await, upd.await, sh.await);
        if !p.alive() {
            return Content::new();
        }
        let sh = sh.unwrap_or_else(common::shell_info);
        let mut out: Content = Vec::new();
        if info.is_none() {
            out.push(common::notice(t("Niet alle informatie is beschikbaar op deze computer.", "Not all information is available on this computer.")).widget().clone().upcast());
        }
        let info = info.unwrap_or(Value::Null);

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
        text.append(&c::label(info["hostname"].as_str().filter(|s| !s.is_empty()).unwrap_or("–"), "st-about-name", true));
        text.append(&c::label(info["deviceModel"].as_str().unwrap_or(""), "st-about-model", true));
        head.append(&text);
        let rename = c::button(t("Naam van deze pc wijzigen", "Rename this PC"), c::Variant::Standard);
        rename.set_valign(gtk::Align::Center);
        rename.set_widget_name("about-rename");
        let (pp, name) = (p.clone(), info["hostname"].as_str().unwrap_or("").to_owned());
        rename.connect_clicked(move |_| rename_hint(&pp, &name));
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
        let dev_rows: Rows = vec![
            (t("Apparaatnaam", "Device name").into(), some(&info["hostname"])),
            (t("Model", "Model").into(), some(&info["deviceModel"])),
            (t("Processor", "Processor").into(), cpu_text),
            (t("Geïnstalleerd RAM", "Installed RAM").into(), info["memory"]["total"].as_f64().filter(|v| *v > 0.0).map(kit::format::bytes)),
            (t("Videokaart", "Graphics card").into(), join(&info["gpus"], &|g: &Value| g.as_str().unwrap_or("").to_owned())),
            (
                t("Opslag", "Storage").into(),
                join(&info["disks"], &|d: &Value| format!("{} ({})", d["model"].as_str().unwrap_or(""), kit::format::bytes(d["size"].as_f64().unwrap_or(0.0))).trim().to_owned()),
            ),
        ];
        let desktop = some(&info["desktop"]).or_else(|| {
            some(&sh["compositor"]).map(|c| {
                let mut ch = c.chars();
                match ch.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                    None => String::new(),
                }
            })
        });
        let shell = some(&sh["version"]).map(|v| {
            let mut s = v;
            if let Some(m) = some(&sh["mode"]) {
                s.push_str(&format!(" · {}", mode_name(&m)));
            }
            if sh["dryRun"] == true {
                s.push_str(&format!(" · {}", t("testmodus", "test mode")));
            }
            s
        });
        let user = &info["user"];
        let user_text = some(&user["name"]).map(|name| format!("{} ({name})", some(&user["realName"]).unwrap_or_else(|| name.clone())));
        let edition = common::os_label(common::s(&info["os"]), common::s(&info["osVersion"]));
        let os_rows: Rows = vec![
            (t("Editie", "Edition").into(), (!edition.is_empty()).then_some(edition)),
            (t("Kernel", "Kernel").into(), some(&info["kernel"])),
            (t("Bureaublad", "Desktop").into(), desktop.filter(|s| !s.is_empty())),
            (t("Win11 Shell", "Win11 Shell").into(), shell),
            (t("Actief sinds opstarten", "Up time").into(), info["uptime"].as_f64().map(kit::format::duration_long)),
            (t("Gebruiker", "User").into(), user_text),
        ];

        let device = SettingsExpander::new("pc", t("Apparaatspecificaties", "Device specifications"), "");
        device.header().set_control(&copy_button(&p, dev_rows.clone(), t("Apparaatspecificaties", "Device specifications")));
        device.add(&spec_list(&dev_rows));
        device.set_expanded(true);
        let os = SettingsExpander::new("info", t("Besturingssysteem", "Operating system"), "");
        os.header().set_control(&copy_button(&p, os_rows.clone(), t("Besturingssysteem", "Operating system")));
        os.add(&spec_list(&os_rows));
        os.set_expanded(true);
        let g = SettingsGroup::new("", "");
        g.add(device.widget());
        g.add(os.widget());
        out.push(g.widget().clone().upcast());

        let related = SettingsGroup::new(t("Gerelateerd", "Related"), "");
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
        out
    })
}
