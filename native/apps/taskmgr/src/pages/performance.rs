//! Prestaties / Performance (js/apps/taskmgr/pages/performance.js, css/taskmgr.css "Prestaties"): the device list
//! with live mini graphs on the left (CPU, memory, every disk, network interface and GPU), the big graph and the
//! Win11 stats grid of the selected device on the right, colour-keyed per device type (taskmgr.colorKeyedGraphs).
//! Weergave: total or per logical processor (CPU) and the graph length (taskmgr.graphSeconds); Kopiëren copies the
//! details as text. The graphs read the store's history, pushing one sample per poll so the grid scrolls.
use super::Page;
use super::storage::layout::{self, Layout};
use crate::app::{App, SettingsSub};
use crate::store::{self, History, Subscription};
use crate::util::{self, Icon, fill};
use kit::chart::{LineChart, LineOptions, Series};
use kit::controls::MenuItem;
use kit::format::{bitrate, bytes, number, number_min, rate};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Prestaties", "Performance"),
        "view" => ("Weergave", "View"),
        "copy" => ("Kopi\u{eb}ren", "Copy"),
        "copied" => ("Gegevens gekopieerd", "Details copied"),
        "graph" => ("Grafiek", "Graph"),
        "overall" => ("Totaal gebruik", "Overall utilisation"),
        "perCore" => ("Per logische processor", "Logical processors"),
        "duration" => ("Grafiekduur", "Graph length"),
        "s30" => ("30 seconden", "30 seconds"),
        "s60" => ("60 seconden", "60 seconds"),
        "s120" => ("2 minuten", "2 minutes"),
        "s300" => ("5 minuten", "5 minutes"),
        "cpu" => ("CPU", "CPU"),
        "memory" => ("Geheugen", "Memory"),
        "disk" => ("Schijf {n}", "Disk {n}"),
        "gpu" => ("GPU {n}", "GPU {n}"),
        "usageOver" => ("% gebruik over {d}", "% Utilisation over {d}"),
        "memUsage" => ("Geheugengebruik", "Memory usage"),
        "memComp" => ("Geheugensamenstelling", "Memory composition"),
        "activeTime" => ("Actieve tijd", "Active time"),
        "transfer" => ("Overdrachtssnelheid schijf", "Disk transfer rate"),
        "throughput" => ("Doorvoer", "Throughput"),
        "gpuMem" => ("Toegewezen GPU-geheugen", "Dedicated GPU memory"),
        "usage" => ("Gebruik", "Utilisation"),
        "speed" => ("Snelheid", "Speed"),
        "processes" => ("Processen", "Processes"),
        "threads" => ("Threads", "Threads"),
        "upTime" => ("Actief", "Up time"),
        "temp" => ("Temperatuur", "Temperature"),
        "cores" => ("Kernen", "Cores"),
        "logical" => ("Logische processors", "Logical processors"),
        "inUse" => ("In gebruik", "In use"),
        "available" => ("Beschikbaar", "Available"),
        "cached" => ("In cache", "Cached"),
        "swap" => ("Wisselbestand", "Swap"),
        "total" => ("Totaal", "Total"),
        "buffers" => ("Buffers", "Buffers"),
        "free" => ("Vrij", "Free"),
        "read" => ("Lezen", "Read"),
        "write" => ("Schrijven", "Write"),
        "readSpeed" => ("Leessnelheid", "Read speed"),
        "writeSpeed" => ("Schrijfsnelheid", "Write speed"),
        "capacity" => ("Capaciteit", "Capacity"),
        "type" => ("Type", "Type"),
        "systemDisk" => ("Systeemschijf", "System disk"),
        "yes" => ("Ja", "Yes"),
        "no" => ("Nee", "No"),
        "mounts" => ("Koppelpunten", "Mount points"),
        "send" => ("Verzenden", "Send"),
        "receive" => ("Ontvangen", "Receive"),
        "adapter" => ("Adapternaam", "Adapter name"),
        "connType" => ("Verbindingstype", "Connection type"),
        "ipv4" => ("IPv4-adres", "IPv4 address"),
        "ipv6" => ("IPv6-adres", "IPv6 address"),
        "gpuUsage" => ("Gebruik", "Utilisation"),
        "gpuMemShort" => ("GPU-geheugen", "GPU memory"),
        "gpuTemp" => ("GPU-temperatuur", "GPU temperature"),
        "vendor" => ("Leverancier", "Vendor"),
        "name" => ("Naam", "Name"),
        "ssd" => ("SSD", "SSD"),
        "hdd" => ("Harde schijf", "HDD"),
        "nvme" => ("SSD (NVMe)", "SSD (NVMe)"),
        "usb" => ("USB-schijf", "USB drive"),
        "ethernet" => ("Ethernet", "Ethernet"),
        "wifi" => ("Wifi", "Wi-Fi"),
        "other" => ("Overig", "Other"),
        "loading" => ("Gegevens verzamelen\u{2026}", "Collecting data\u{2026}"),
        "unknown" => ("Onbekend", "Unknown"),
        "notAvailable" => ("Niet beschikbaar", "Not available"),
        "paused" => ("Bijwerken is gepauzeerd (zie Instellingen)", "Updating is paused (see Settings)"),
        "send_s" => ("Verz.:", "S:"),
        "recv_s" => ("Ontv.:", "R:"),
        "baseSpeed" => ("Basissnelheid", "Base speed"),
        "maxSpeed" => ("Maximale snelheid", "Maximum speed"),
        "committed" => ("Toegewezen", "Committed"),
        "response" => ("Gem. reactietijd", "Average response time"),
        "linkSpeed" => ("Verbindingssnelheid", "Link speed"),
        "iface" => ("Interface", "Interface"),
        "driver" => ("Stuurprogramma", "Driver"),
        "power" => ("Vermogen", "Power"),
        "clock" => ("Kloksnelheid", "Clock speed"),
        "e3d" => ("3D", "3D"),
        "encode" => ("Video coderen", "Video encode"),
        "decode" => ("Video decoderen", "Video decode"),
        "disconnected" => ("Niet verbonden", "Not connected"),
        "device" => ("Apparaat", "Device"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

/// A device kind as text ("ssd" -> "SSD", "wifi" -> "Wifi"); unknown kinds as they are.
fn kind_text(kind: &str) -> String {
    match kind {
        "ssd" | "hdd" | "nvme" | "usb" | "ethernet" | "wifi" | "other" => tr(kind).to_owned(),
        _ => kind.to_owned(),
    }
}

const DURATIONS: [u64; 4] = [30, 60, 120, 300];
const DASH: &str = "\u{2013}";
static EMPTY: VecDeque<f64> = VecDeque::new();

fn dur_label(s: u64) -> String {
    match s {
        30 => tr("s30").to_owned(),
        60 => tr("s60").to_owned(),
        120 => tr("s120").to_owned(),
        300 => tr("s300").to_owned(),
        _ => format!("{s} s"),
    }
}

/// "227 Kbps", "1,2 Mbps": compact for the device list.
fn short_rate(bps: f64) -> String {
    let mut v = if bps.is_finite() { bps * 8.0 } else { 0.0 };
    let units = ["bps", "Kbps", "Mbps", "Gbps"];
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{} {}", number(v, if v >= 100.0 || i == 0 { 0 } else { 1 }), units[i])
}

/// "NVIDIA GeForce RTX 3060" -> "RTX 3060".
fn short_gpu(name: &str) -> String {
    let Some(rest) = ["nvidia", "amd", "intel(r)", "intel"].iter().find_map(|p| layout::strip_word(name, p)) else { return name.to_owned() };
    let rest = ["geforce", "radeon"].iter().find_map(|q| layout::strip_word(rest, q)).unwrap_or(rest);
    if rest.is_empty() { name.to_owned() } else { rest.to_owned() }
}

/// A rounded-up "nice" maximum for auto-scaled graphs (bytes/s).
fn nice_max(v: f64, minimum: f64) -> f64 {
    let x = minimum.max(v * 1.15);
    let p = 10f64.powf(x.log10().floor());
    let m = x / p;
    let step = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * p
}

fn max_of(series: &[Vec<f64>]) -> f64 {
    series.iter().flatten().copied().filter(|v| v.is_finite()).fold(1.0, f64::max)
}

fn temp(v: &Value) -> Option<String> {
    v.as_f64().filter(|t| t.is_finite()).map(|t| format!("{} \u{b0}C", t.round() as i64))
}

/// rate / bitrate without a ",0" (performance.js `.replace(/,0 /, ' ')`).
fn trim_zero(s: String) -> String {
    s.replacen(",0 ", " ", 1)
}

#[derive(Clone)]
struct Dev {
    id: String,
    kind: &'static str,
    index: usize,
    name: String,
    iface: String,
}

fn device_list(p: &Value) -> Vec<Dev> {
    let mut list = vec![
        Dev { id: "cpu".into(), kind: "cpu", index: 0, name: String::new(), iface: String::new() },
        Dev { id: "memory".into(), kind: "memory", index: 0, name: String::new(), iface: String::new() },
    ];
    for (i, d) in p["disks"].as_array().into_iter().flatten().enumerate() {
        let name = util::text(&d["name"]);
        let index = d["index"].as_u64().map(|n| n as usize).unwrap_or(i);
        list.push(Dev { id: format!("disk:{name}"), kind: "disk", index, name, iface: String::new() });
    }
    for n in p["network"].as_array().into_iter().flatten() {
        let iface = util::text(&n["iface"]);
        list.push(Dev { id: format!("net:{iface}"), kind: "network", index: 0, name: String::new(), iface });
    }
    for (i, _) in p["gpus"].as_array().into_iter().flatten().enumerate() {
        list.push(Dev { id: format!("gpu:{i}"), kind: "gpu", index: i, name: String::new(), iface: String::new() });
    }
    list
}

/// What a chart was fed last: (history count, points, series).
type Fed = Rc<Cell<Option<(u64, usize, usize)>>>;

struct Tile {
    dev: Dev,
    button: gtk::Button,
    chart: LineChart,
    fed: Fed,
    title: gtk::Label,
    sub1: gtk::Label,
    sub2: gtk::Label,
    max: Cell<f64>,
}

/// One figure of the stats grid (`.tm-stat`).
#[derive(Clone)]
struct Stat {
    root: gtk::Box,
    label: gtk::Label,
    value: gtk::Label,
}

impl Stat {
    fn new(label: &str, big: bool) -> Stat {
        let root = layout::vbox(0, "taskmgr-stat");
        root.set_size_request(96, -1);
        let l = layout::label(label, "taskmgr-stat-label");
        let v = layout::label(DASH, if big { "taskmgr-stat-value taskmgr-stat-big" } else { "taskmgr-stat-value" });
        v.set_ellipsize(gtk::pango::EllipsizeMode::None);
        root.append(&l);
        root.append(&v);
        Stat { root, label: l, value: v }
    }

    fn set(&self, s: &str) {
        if self.value.label() != s {
            self.value.set_label(s);
        }
    }

    fn show(&self, on: bool) {
        self.root.set_visible(on);
    }
}

/// One line of the key / value list (`.tm-kv`).
#[derive(Clone)]
struct Kv {
    row: gtk::Box,
    key: gtk::Label,
    value: gtk::Label,
}

impl Kv {
    fn new(key: &str, group: &gtk::SizeGroup) -> Kv {
        let row = layout::hbox(24, "taskmgr-kv-row");
        let k = layout::label(key, "taskmgr-kv-key");
        k.set_ellipsize(gtk::pango::EllipsizeMode::None);
        k.set_valign(gtk::Align::Start);
        group.add_widget(&k);
        let v = layout::wrap(DASH, "taskmgr-kv-value");
        v.set_hexpand(true);
        row.append(&k);
        row.append(&v);
        Kv { row, key: k, value: v }
    }

    fn set(&self, s: &str) {
        if self.value.label() != s {
            self.value.set_label(s);
        }
    }

    fn show(&self, on: bool) {
        self.row.set_visible(on);
    }
}

/// The detail header: the device name and, on the right, its model (`.tm-detail__head`).
#[derive(Clone)]
struct Head {
    root: gtk::Box,
    title: gtk::Label,
    model: gtk::Label,
}

impl Head {
    fn new(title: &str, model: &str) -> Head {
        let root = layout::hbox(16, "taskmgr-detail-head");
        let t = layout::label(title, "taskmgr-detail-title");
        t.set_ellipsize(gtk::pango::EllipsizeMode::None);
        t.set_valign(gtk::Align::End);
        let m = layout::label(model, "taskmgr-detail-model");
        m.set_xalign(1.0);
        m.set_hexpand(true);
        m.set_valign(gtk::Align::End);
        root.append(&t);
        root.append(&m);
        Head { root, title: t, model: m }
    }

    fn set_title(&self, s: &str) {
        if self.title.label() != s {
            self.title.set_label(s);
        }
    }

    fn set_right(&self, s: &str) {
        if self.model.label() != s {
            self.model.set_label(s);
        }
    }
}

/// A graph block (`.tm-graph`): the labels above and below, the chart in between.
#[derive(Clone)]
struct Graph {
    root: gtk::Widget,
    right: gtk::Label,
    chart: LineChart,
    fed: Fed,
}

impl Graph {
    fn set_right(&self, s: &str) {
        if self.right.label() != s {
            self.right.set_label(s);
        }
    }
}

/// The page's detail panel for one device: its widgets and how they follow the data.
struct Detail {
    root: gtk::Box,
    head: Head,
    stats: Vec<Stat>,
    kvs: Vec<Kv>,
    update: Box<dyn Fn(&Perf, &Value)>,
}

impl Detail {
    /// performance.js copySummary: the title, the model, every figure and every key / value as lines.
    fn lines(&self) -> Vec<String> {
        let mut out = vec![self.head.title.label().to_string()];
        let model = self.head.model.label().to_string();
        if !model.is_empty() {
            out.push(model);
        }
        for s in self.stats.iter().filter(|s| s.root.is_visible()) {
            out.push(format!("{}: {}", s.label.label(), s.value.label()));
        }
        for k in self.kvs.iter().filter(|k| k.row.is_visible()) {
            out.push(format!("{}: {}", k.key.label(), k.value.label()));
        }
        out
    }
}

struct Perf {
    me: Weak<Perf>,
    app: Rc<App>,
    root: gtk::Box,
    list: gtk::Box,
    panel: gtk::Box,
    perf: RefCell<Option<Rc<Value>>>,
    selected: RefCell<String>,
    cpu_mode: RefCell<String>,
    tiles: RefCell<Vec<Rc<Tile>>>,
    tile_ids: RefCell<String>,
    detail: RefCell<Option<Rc<Detail>>>,
    vol_by_mount: RefCell<Option<HashMap<String, Value>>>,
    vols_failed: Cell<bool>,
    subs: RefCell<Vec<Subscription>>,
    settings_sub: RefCell<Option<SettingsSub>>,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|me: &Weak<Perf>| build(app, me.clone()));
    page.wire();
    page
}

fn build(app: &Rc<App>, me: Weak<Perf>) -> Perf {
    let list = layout::vbox(2, "taskmgr-perf-list");
    let list_scroll = kit::controls::scrolled(&list);
    list_scroll.set_hexpand(false);
    list_scroll.set_size_request(264, -1);
    list_scroll.set_widget_name("taskmgr-perf-list");
    let panel = layout::vbox(0, "taskmgr-perf-panel");
    panel.set_hexpand(true);
    let loading = layout::label(tr("loading"), "taskmgr-perf-loading");
    loading.set_xalign(0.5);
    panel.append(&loading);
    let panel_scroll = gtk::ScrolledWindow::new();
    panel_scroll.set_hexpand(true);
    panel_scroll.set_vexpand(true);
    panel_scroll.set_child(Some(&panel));
    let root = layout::hbox(0, "taskmgr-perf");
    root.set_vexpand(true);
    root.append(&list_scroll);
    root.append(&panel_scroll);

    let new_btn = super::new_task_button(app);
    let copy_btn = util::cmd_button(Icon::Glyph("copy"), tr("copy"));
    copy_btn.set_widget_name("taskmgr-copy");
    let view_btn = util::cmd_button(Icon::Glyph("view"), tr("view"));
    view_btn.set_widget_name("taskmgr-view");
    if let Some(row) = view_btn.child().and_downcast::<gtk::Box>() {
        let chev = kit::icons::glyph("chevron-down", 12);
        chev.add_css_class("taskmgr-cmd-chevron");
        row.append(&chev);
    }
    let w = me.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.copy_summary();
        }
    });
    let w = me.clone();
    view_btn.connect_clicked(move |b| {
        if let Some(p) = w.upgrade() {
            p.view_menu(b);
        }
    });
    let commands = vec![new_btn.upcast(), util::cmd_sep(), copy_btn.upcast(), view_btn.upcast()];
    let selected = util::load("perf.device").as_str().unwrap_or("cpu").to_owned();
    let cpu_mode = util::load("perf.cpuMode").as_str().unwrap_or("overall").to_owned();
    Perf {
        me,
        app: app.clone(),
        root,
        list,
        panel,
        perf: RefCell::new(None),
        selected: RefCell::new(selected),
        cpu_mode: RefCell::new(cpu_mode),
        tiles: RefCell::new(Vec::new()),
        tile_ids: RefCell::new(String::new()),
        detail: RefCell::new(None),
        vol_by_mount: RefCell::new(None),
        vols_failed: Cell::new(false),
        subs: RefCell::new(Vec::new()),
        settings_sub: RefCell::new(None),
        commands,
    }
}

impl Perf {
    fn wire(&self) {
        // Up / Down / Home / End move through the devices
        let keys = gtk::EventControllerKey::new();
        let w = self.me.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            let ids: Vec<String> = p.tiles.borrow().iter().map(|t| t.dev.id.clone()).collect();
            if ids.is_empty() {
                return glib::Propagation::Proceed;
            }
            let i = ids.iter().position(|x| *x == *p.selected.borrow()).unwrap_or(0);
            let j = match key {
                gdk::Key::Down | gdk::Key::KP_Down => (i + 1).min(ids.len() - 1),
                gdk::Key::Up | gdk::Key::KP_Up => i.saturating_sub(1),
                gdk::Key::Home | gdk::Key::KP_Home => 0,
                gdk::Key::End | gdk::Key::KP_End => ids.len() - 1,
                _ => return glib::Propagation::Proceed,
            };
            p.select(&ids[j], true);
            glib::Propagation::Stop
        });
        self.list.add_controller(keys);

        // graph colours, durations and the update speed follow the settings
        let w = self.me.clone();
        let sub = self.app.on_settings(move |_, path| {
            let Some(p) = w.upgrade() else { return };
            if path.is_some_and(|path| !path.starts_with("taskmgr")) {
                return;
            }
            let Some(perf) = p.perf.borrow().clone() else { return };
            p.tile_ids.borrow_mut().clear();
            p.build_list(&perf);
            p.update_list(&perf);
            p.build_detail();
        });
        *self.settings_sub.borrow_mut() = Some(sub);
    }

    // ------------------------------------------------------------------------------------- graph feeding

    fn graph_seconds(&self) -> u64 {
        self.app.setting("taskmgr.graphSeconds").as_u64().filter(|s| *s > 0).unwrap_or(60)
    }

    /// Samples across a graph: the graph length at the update speed.
    fn points(&self) -> usize {
        let speed = match self.app.store.speed_ms() {
            0 => 1000,
            ms => ms,
        };
        (((self.graph_seconds() * 1000) as f64 / speed as f64).round() as usize + 1).max(8)
    }

    /// Feed a chart from history series: push the newest sample (the grid scrolls) or redraw all of it.
    fn feed(&self, chart: &LineChart, fed: &Fed, dashed: bool, pick: impl for<'a> Fn(&'a History) -> Vec<&'a VecDeque<f64>>) {
        let h = self.app.store.history();
        let srcs = pick(&h);
        let (count, pts, len) = (h.count, self.points(), srcs.len());
        match fed.get() {
            Some((c, p, _)) if c == count && p == pts => return,
            Some((c, p, l)) if c + 1 == count && p == pts && l == len => {
                let last: Vec<f64> = srcs.iter().map(|a| a.back().copied().unwrap_or(0.0)).collect();
                chart.push(&last);
            }
            _ => {
                chart.set_points(pts);
                chart.update(
                    srcs.iter()
                        .enumerate()
                        .map(|(i, a)| {
                            let mut s = Series::new(store::tail(a, pts));
                            s.dashed = dashed && i == 1;
                            s
                        })
                        .collect(),
                );
            }
        }
        fed.set(Some((count, pts, len)));
    }

    /// The last graph-length of two series (for the auto-scaled maximum).
    fn tails(&self, pick: impl for<'a> Fn(&'a History) -> Vec<&'a VecDeque<f64>>) -> Vec<Vec<f64>> {
        let h = self.app.store.history();
        let pts = self.points();
        pick(&h).into_iter().map(|s| store::tail(s, pts)).collect()
    }

    // ------------------------------------------------------------------------------------- drive names

    /// "Schijf 0 (Systeemschijf, Virtuele machines)": the names Verkenner and Instellingen show for its
    /// mount points instead of raw folders or UUIDs.
    fn disk_title(&self, dev: &Dev, d: &Value) -> String {
        let mounts: Vec<String> = d["mounts"].as_array().into_iter().flatten().map(util::text).collect();
        let mut names: Vec<String> = Vec::new();
        if let Some(map) = self.vol_by_mount.borrow().as_ref() {
            for m in &mounts {
                if let Some(v) = map.get(m).filter(|v| v["hidden"] != true) {
                    let n = [&v["name"], &v["label"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
                    if !n.is_empty() {
                        names.push(n);
                    }
                }
            }
        } else if self.vols_failed.get() {
            for m in &mounts {
                let last = m.rsplit('/').next().unwrap_or("");
                let uuid = last.len() >= 8 && last.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
                if !uuid {
                    names.push(if m == "/" { "/".into() } else { last.to_owned() });
                }
            }
        }
        let mut seen = Vec::new();
        names.retain(|n| {
            if seen.contains(n) {
                false
            } else {
                seen.push(n.clone());
                true
            }
        });
        let base = fill(tr("disk"), &[("n", &dev.index.to_string())]);
        if names.is_empty() { base } else { format!("{base} ({})", names.join(", ")) }
    }

    // --------------------------------------------------------------------------------------- left list

    fn tile_texts(&self, dev: &Dev, p: &Value) -> (String, String, String) {
        match dev.kind {
            "cpu" => {
                let c = &p["cpu"];
                (tr("cpu").into(), format!("{}%  {}", util::num(&c["usage"]).round() as i64, util::ghz(util::num(&c["freqMHz"]))), temp(&c["temperature"]).unwrap_or_default())
            }
            "memory" => {
                let m = &p["memory"];
                let (used, total) = (util::num(&m["used"]), util::num(&m["total"]));
                let pct = if total > 0.0 { (used / total * 100.0).round() as i64 } else { 0 };
                (tr("memory").into(), format!("{}/{} ({pct}%)", kit::format::bytes_digits(used, Some(1)), kit::format::bytes_digits(total, Some(1))), String::new())
            }
            "disk" => match p["disks"].as_array().and_then(|a| a.iter().find(|x| util::text(&x["name"]) == dev.name)) {
                Some(d) => (self.disk_title(dev, d), kind_text(&util::text(&d["kind"])), format!("{}%", util::num(&d["activePct"]).round() as i64)),
                None => (fill(tr("disk"), &[("n", &dev.index.to_string())]), String::new(), String::new()),
            },
            "network" => match p["network"].as_array().and_then(|a| a.iter().find(|x| util::text(&x["iface"]) == dev.iface)) {
                Some(n) => {
                    let name = [&n["name"], &n["iface"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
                    if n["connected"] == false {
                        (name, util::text(&n["iface"]), tr("disconnected").into())
                    } else {
                        (name, format!("{} {}", tr("send_s"), short_rate(util::num(&n["txBps"]))), format!("{} {}", tr("recv_s"), short_rate(util::num(&n["rxBps"]))))
                    }
                }
                None => Default::default(),
            },
            "gpu" => match p["gpus"].as_array().and_then(|a| a.get(dev.index)) {
                Some(g) => {
                    let t = temp(&g["temperature"]).map(|t| format!(" ({t})")).unwrap_or_default();
                    let usage = match g["usage"].as_f64() {
                        Some(u) => format!("{}%{t}", u.round() as i64),
                        None => tr("notAvailable").into(),
                    };
                    (fill(tr("gpu"), &[("n", &dev.index.to_string())]), short_gpu(&util::text(&g["name"])), usage)
                }
                None => Default::default(),
            },
            _ => Default::default(),
        }
    }

    fn build_list(&self, p: &Value) {
        let devs = device_list(p);
        let ids = devs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>().join("|");
        if *self.tile_ids.borrow() == ids {
            return;
        }
        *self.tile_ids.borrow_mut() = ids;
        kit::controls::clear(&self.list);
        let mut tiles = Vec::new();
        for dev in devs {
            let chart = LineChart::new(LineOptions {
                max: 100.0,
                color: util::kind_color(dev.kind).into(),
                gridlines: false,
                points: self.points(),
                fill_alpha: 0.18,
                ..Default::default()
            });
            let cw = chart.widget();
            cw.set_size_request(72, 48);
            cw.set_hexpand(false);
            cw.set_vexpand(false);
            cw.set_valign(gtk::Align::Center);
            cw.add_css_class("taskmgr-graph-box");
            let title = layout::label("", "taskmgr-tile-title");
            let sub1 = layout::label("", "taskmgr-tile-sub");
            let sub2 = layout::label("", "taskmgr-tile-sub");
            let text = layout::vbox(1, "taskmgr-tile-text");
            text.set_valign(gtk::Align::Center);
            text.set_hexpand(true);
            text.append(&title);
            text.append(&sub1);
            text.append(&sub2);
            let row = layout::hbox(12, "");
            row.append(cw);
            row.append(&text);
            let button = layout::flat_button("taskmgr-tile", &row);
            button.set_widget_name(&format!("taskmgr-device-{}", dev.id));
            let w = self.me.clone();
            let id = dev.id.clone();
            button.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.select(&id, true);
                }
            });
            self.list.append(&button);
            tiles.push(Rc::new(Tile { dev, button, chart, fed: Rc::new(Cell::new(None)), title, sub1, sub2, max: Cell::new(100.0) }));
        }
        *self.tiles.borrow_mut() = tiles;
        let known = self.tiles.borrow().iter().any(|t| t.dev.id == *self.selected.borrow());
        if !known {
            *self.selected.borrow_mut() = "cpu".into();
        }
        self.mark_selected();
    }

    fn update_list(&self, p: &Value) {
        let tiles = self.tiles.borrow().clone();
        for t in tiles {
            let (a, b, c) = self.tile_texts(&t.dev, p);
            if t.title.label() != a {
                t.title.set_label(&a);
                // long drive names are cut off in the narrow list: the full name on hover
                if t.dev.kind == "disk" {
                    t.title.set_tooltip_text(Some(&a));
                }
            }
            if t.sub1.label() != b {
                t.sub1.set_label(&b);
            }
            if t.sub2.label() != c {
                t.sub2.set_label(&c);
            }
            t.sub2.set_visible(!c.is_empty());
            let dev = t.dev.clone();
            let mut max = 100.0;
            if dev.kind == "network" {
                let iface = dev.iface.clone();
                let tails = self.tails(|h| match h.nets.get(&iface) {
                    Some(n) => vec![&n.rx, &n.tx],
                    None => vec![&EMPTY, &EMPTY],
                });
                max = nice_max(max_of(&tails), 12500.0);
            }
            if t.max.get() != max {
                t.max.set(max);
                t.chart.set_max(max);
            }
            match dev.kind {
                "cpu" => self.feed(&t.chart, &t.fed, true, |h| vec![&h.cpu]),
                "memory" => self.feed(&t.chart, &t.fed, true, |h| vec![&h.mem]),
                "disk" => self.feed(&t.chart, &t.fed, true, |h| vec![h.disks.get(&dev.name).map(|d| &d.active).unwrap_or(&EMPTY)]),
                "network" => self.feed(&t.chart, &t.fed, true, |h| match h.nets.get(&dev.iface) {
                    Some(n) => vec![&n.rx, &n.tx],
                    None => vec![&EMPTY, &EMPTY],
                }),
                "gpu" => self.feed(&t.chart, &t.fed, true, |h| vec![h.gpus.get(&dev.index).map(|g| &g.usage).unwrap_or(&EMPTY)]),
                _ => {}
            }
        }
    }

    fn mark_selected(&self) {
        let sel = self.selected.borrow().clone();
        for t in self.tiles.borrow().iter() {
            util::set_class(&t.button, "taskmgr-selected", t.dev.id == sel);
        }
    }

    fn select(&self, id: &str, user: bool) {
        let tile = self.tiles.borrow().iter().find(|t| t.dev.id == id).cloned();
        let Some(tile) = tile else { return };
        *self.selected.borrow_mut() = id.to_owned();
        util::save("perf.device", json!(id));
        self.mark_selected();
        self.build_detail();
        if user {
            tile.button.grab_focus();
        }
    }

    // ------------------------------------------------------------------------------------ detail panel

    fn graph_frame(&self, left: &str, right: &str, body: &impl IsA<gtk::Widget>, grow: bool) -> (gtk::Widget, gtk::Label) {
        let top = layout::hbox(16, "taskmgr-graph-labels");
        let l = layout::label(left, "taskmgr-graph-lab");
        l.set_hexpand(true);
        let r = layout::label(right, "taskmgr-graph-lab");
        r.set_xalign(1.0);
        top.append(&l);
        top.append(&r);
        let bottom = layout::hbox(16, "taskmgr-graph-labels");
        let d = layout::label(&dur_label(self.graph_seconds()), "taskmgr-graph-lab");
        d.set_hexpand(true);
        let z = layout::label("0", "taskmgr-graph-lab");
        z.set_xalign(1.0);
        bottom.append(&d);
        bottom.append(&z);
        let block = layout::vbox(4, "taskmgr-graph");
        body.as_ref().set_vexpand(true);
        block.append(&top);
        block.append(body.as_ref());
        block.append(&bottom);
        // .tm-graph--grow: min 180 px, at most 480 px; .tm-graph--half: 140 px, at least 110 px
        let (min, nat) = if grow { (180, 480) } else { (110, 140) };
        let wrap = Layout::columns(0, 0, |w| vec![w]).heights(min, nat);
        wrap.append(&block);
        (wrap.upcast(), r)
    }

    fn graph(&self, left: &str, right: &str, color: &str, grow: bool, max: f64, grid_rows: u32) -> Graph {
        let chart = LineChart::new(LineOptions { max, color: color.into(), points: self.points(), grid_rows, ..Default::default() });
        chart.widget().add_css_class("taskmgr-graph-box");
        let (root, right) = self.graph_frame(left, right, chart.widget(), grow);
        Graph { root, right, chart, fed: Rc::new(Cell::new(None)) }
    }

    fn stats_block(&self, rows: &[Vec<&Stat>], kvs: &[&Kv]) -> gtk::Widget {
        let main = layout::vbox(12, "taskmgr-stats-main");
        main.set_size_request(260, -1);
        for r in rows {
            let row = layout::hbox(32, "taskmgr-stats-row");
            for s in r {
                row.append(&s.root);
            }
            main.append(&row);
        }
        let kv = layout::vbox(4, "taskmgr-kv");
        for k in kvs {
            kv.append(&k.row);
        }
        let stats = Layout::wrap(48, 16);
        stats.add_css_class("taskmgr-stats");
        stats.append(&main);
        stats.append(&kv);
        stats.upcast()
    }

    fn legend(&self, color: &str, a: &str, b: &str) -> gtk::Box {
        let legend = layout::hbox(20, "taskmgr-legend");
        for (text, dashed) in [(a, false), (b, true)] {
            let key = layout::hbox(8, "");
            key.append(&layout::legend_line(color, dashed));
            key.append(&layout::label(text, ""));
            legend.append(&key);
        }
        legend
    }

    fn detail_root(&self) -> gtk::Box {
        let root = layout::vbox(12, "taskmgr-detail");
        root.set_size_request(-1, 520);
        root.set_vexpand(true);
        root
    }

    fn build_detail(&self) {
        kit::controls::clear(&self.panel);
        self.detail.borrow_mut().take();
        let Some(perf) = self.perf.borrow().clone() else {
            let l = layout::label(tr("loading"), "taskmgr-perf-loading");
            l.set_xalign(0.5);
            self.panel.append(&l);
            return;
        };
        let dev = self
            .tiles
            .borrow()
            .iter()
            .find(|t| t.dev.id == *self.selected.borrow())
            .map(|t| t.dev.clone())
            .unwrap_or(Dev { id: "cpu".into(), kind: "cpu", index: 0, name: String::new(), iface: String::new() });
        let color = util::kind_color(dev.kind);
        let detail = Rc::new(match dev.kind {
            "memory" => self.mem_detail(color),
            "disk" => self.disk_detail(&dev, color),
            "network" => self.net_detail(&dev, color),
            "gpu" => self.gpu_detail(&dev, color),
            _ => self.cpu_detail(&perf, color),
        });
        self.panel.append(&detail.root);
        (detail.update)(self, &perf);
        *self.detail.borrow_mut() = Some(detail);
        if self.app.store.paused() {
            let l = layout::label(tr("paused"), "taskmgr-perf-paused w11-caption");
            self.panel.append(&l);
        }
    }

    fn cpu_detail(&self, perf: &Value, color: &str) -> Detail {
        let head = Head::new(tr("cpu"), &util::text(&perf["cpu"]["model"]));
        let usage_over = fill(tr("usageOver"), &[("d", &dur_label(self.graph_seconds()))]);
        let n = perf["cpu"]["perCore"].as_array().map(|a| a.len()).unwrap_or(0);
        let mut charts: Vec<(LineChart, Fed)> = Vec::new();
        let graph_area = if *self.cpu_mode.borrow() == "perCore" && n > 0 {
            let cols = if n <= 4 { 2 } else if n <= 16 { 4 } else { 8 };
            let grid = gtk::Grid::new();
            grid.add_css_class("taskmgr-cores");
            grid.set_row_spacing(6);
            grid.set_column_spacing(6);
            grid.set_row_homogeneous(true);
            grid.set_column_homogeneous(true);
            for i in 0..n {
                let c = LineChart::new(LineOptions { max: 100.0, color: color.into(), points: self.points(), grid_rows: 4, ..Default::default() });
                c.widget().add_css_class("taskmgr-graph-box");
                c.widget().set_size_request(-1, 40);
                grid.attach(c.widget(), (i % cols) as i32, (i / cols) as i32, 1, 1);
                charts.push((c, Rc::new(Cell::new(None))));
            }
            self.graph_frame(&usage_over, "100%", &grid, true).0
        } else {
            let g = self.graph(&usage_over, "100%", color, true, 100.0, 0);
            charts.push((g.chart.clone(), g.fed.clone()));
            g.root
        };
        let s_use = Stat::new(tr("usage"), true);
        let s_speed = Stat::new(tr("speed"), true);
        let s_proc = Stat::new(tr("processes"), false);
        let s_thr = Stat::new(tr("threads"), false);
        let s_up = Stat::new(tr("upTime"), false);
        let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let k_base = Kv::new(tr("baseSpeed"), &group);
        let k_max = Kv::new(tr("maxSpeed"), &group);
        let k_cores = Kv::new(tr("cores"), &group);
        let k_log = Kv::new(tr("logical"), &group);
        let k_temp = Kv::new(tr("temp"), &group);
        let root = self.detail_root();
        root.append(&head.root);
        root.append(&graph_area);
        root.append(&self.stats_block(&[vec![&s_use, &s_speed], vec![&s_proc, &s_thr], vec![&s_up]], &[&k_base, &k_max, &k_cores, &k_log, &k_temp]));
        let (h, su, ssp, sp, st, sup) = (head.clone(), s_use.clone(), s_speed.clone(), s_proc.clone(), s_thr.clone(), s_up.clone());
        let (kb, km, kc, kl, kt) = (k_base.clone(), k_max.clone(), k_cores.clone(), k_log.clone(), k_temp.clone());
        let update = Box::new(move |page: &Perf, p: &Value| {
            let c = &p["cpu"];
            h.set_right(&util::text(&c["model"]));
            if charts.len() > 1 {
                for (i, (ch, fed)) in charts.iter().enumerate() {
                    page.feed(ch, fed, false, |hist| vec![hist.cpu_cores.get(i).unwrap_or(&EMPTY)]);
                }
            } else if let Some((ch, fed)) = charts.first() {
                page.feed(ch, fed, false, |hist| vec![&hist.cpu]);
            }
            su.set(&format!("{}%", util::num(&c["usage"]).round() as i64));
            ssp.set(&util::ghz(util::num(&c["freqMHz"])));
            sp.set(&number(util::num(&c["processes"]), 0));
            st.set(&number(util::num(&c["threadsTotal"]), 0));
            sup.set(&kit::format::uptime(util::num(&c["uptime"])));
            kc.set(&c["cores"].as_f64().map(|v| number(v, 0)).unwrap_or_else(|| DASH.into()));
            kl.set(&c["threads"].as_f64().map(|v| number(v, 0)).unwrap_or_else(|| DASH.into()));
            kt.set(&temp(&c["temperature"]).unwrap_or_else(|| tr("notAvailable").into()));
            let base = util::num(&c["baseMHz"]);
            kb.show(base > 0.0);
            if base > 0.0 {
                kb.set(&util::ghz(base));
            }
            let max = util::num(&c["maxMHz"]);
            km.show(max > 0.0);
            if max > 0.0 {
                km.set(&util::ghz(max));
            }
        });
        Detail { root, head, stats: vec![s_use, s_speed, s_proc, s_thr, s_up], kvs: vec![k_base, k_max, k_cores, k_log, k_temp], update }
    }

    fn mem_detail(&self, color: &str) -> Detail {
        let head = Head::new(tr("memory"), "");
        let g = self.graph(tr("memUsage"), "", color, true, 100.0, 0);
        // the composition bar: in use, cached, free (.tm-memcomp)
        let parts: Rc<RefCell<[(f64, String); 3]>> = Rc::new(RefCell::new(Default::default()));
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(40);
        bar.set_hexpand(true);
        bar.add_css_class("taskmgr-memcomp-bar");
        let kind = color.to_owned();
        let pw = Rc::downgrade(&parts);
        bar.set_draw_func(move |_, cr, w, h| {
            let Some(parts) = pw.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let k = layout::color(&kind);
            let mut x = 0.0;
            for (i, (frac, _)) in parts.borrow().iter().enumerate() {
                let sw = frac * w;
                let a = match i {
                    0 => 0.32,
                    1 => 0.12,
                    _ => 0.0,
                };
                if a > 0.0 && sw > 0.0 {
                    cr.set_source_rgba(k.red() as f64, k.green() as f64, k.blue() as f64, k.alpha() as f64 * a);
                    cr.rectangle(x, 0.0, sw, h);
                    cr.fill().ok();
                }
                if i > 0 {
                    layout::set_source(cr, &k);
                    cr.rectangle(x.round(), 0.0, 1.0, h);
                    cr.fill().ok();
                }
                x += sw;
            }
            layout::set_source(cr, &k);
            cr.set_line_width(1.0);
            cr.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
            cr.stroke().ok();
        });
        bar.set_has_tooltip(true);
        let pw = Rc::downgrade(&parts);
        bar.connect_query_tooltip(move |a, x, _, _, tip| {
            let Some(parts) = pw.upgrade() else { return false };
            let w = a.width() as f64;
            let mut at = 0.0;
            for (frac, text) in parts.borrow().iter() {
                if (x as f64) >= at && (x as f64) < at + frac * w {
                    tip.set_text(Some(text));
                    return true;
                }
                at += frac * w;
            }
            false
        });
        kit::theme::track(&bar);
        let comp = layout::vbox(4, "taskmgr-graph taskmgr-memcomp");
        let top = layout::hbox(16, "taskmgr-graph-labels");
        top.append(&layout::label(tr("memComp"), "taskmgr-graph-lab"));
        comp.append(&top);
        comp.append(&bar);
        let s_use = Stat::new(tr("inUse"), true);
        let s_avail = Stat::new(tr("available"), true);
        let s_cache = Stat::new(tr("cached"), false);
        let s_swap = Stat::new(tr("swap"), false);
        let s_commit = Stat::new(tr("committed"), false);
        let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let k_total = Kv::new(tr("total"), &group);
        let k_buf = Kv::new(tr("buffers"), &group);
        let root = self.detail_root();
        root.append(&head.root);
        root.append(&g.root);
        root.append(&comp);
        root.append(&self.stats_block(&[vec![&s_use, &s_avail], vec![&s_commit, &s_cache, &s_swap]], &[&k_total, &k_buf]));
        let (h, gg, su, sa, sc, ss, scm, kt, kb) =
            (head.clone(), g.clone(), s_use.clone(), s_avail.clone(), s_cache.clone(), s_swap.clone(), s_commit.clone(), k_total.clone(), k_buf.clone());
        let update = Box::new(move |page: &Perf, p: &Value| {
            let m = &p["memory"];
            let total = util::num(&m["total"]);
            let b1 = |v: f64| kit::format::bytes_digits(v, Some(1));
            h.set_right(&b1(total));
            gg.set_right(&b1(total));
            page.feed(&gg.chart, &gg.fed, false, |hist| vec![&hist.mem]);
            let cache = util::num(&m["cached"]) + util::num(&m["buffers"]);
            let used = util::num(&m["used"]);
            let free = (total - used - cache).max(0.0);
            if total > 0.0 {
                *parts.borrow_mut() = [
                    (used / total, format!("{}: {}", tr("inUse"), bytes(used))),
                    (cache.min(total - used).max(0.0) / total, format!("{}: {}", tr("cached"), bytes(cache))),
                    (free / total, format!("{}: {}", tr("free"), bytes(free))),
                ];
                bar.queue_draw();
            }
            su.set(&b1(used));
            sa.set(&b1(util::num(&m["available"])));
            sc.set(&b1(cache));
            let swap_total = util::num(&m["swapTotal"]);
            ss.set(&if swap_total > 0.0 { format!("{}/{}", b1(util::num(&m["swapUsed"])), b1(swap_total)) } else { tr("notAvailable").into() });
            kt.set(&b1(total));
            kb.set(&bytes(util::num(&m["buffers"])));
            let (committed, limit) = (util::num(&m["committed"]), util::num(&m["commitLimit"]));
            scm.show(committed > 0.0 && limit > 0.0);
            if committed > 0.0 && limit > 0.0 {
                scm.set(&format!("{}/{}", b1(committed), b1(limit)));
            }
        });
        Detail { root, head, stats: vec![s_use, s_avail, s_commit, s_cache, s_swap], kvs: vec![k_total, k_buf], update }
    }

    fn disk_detail(&self, dev: &Dev, color: &str) -> Detail {
        let head = Head::new(&fill(tr("disk"), &[("n", &dev.index.to_string())]), "");
        let g1 = self.graph(tr("activeTime"), "100%", color, true, 100.0, 0);
        let g2 = self.graph(tr("transfer"), "", color, false, 1_048_576.0, 4);
        let legend = self.legend(color, tr("read"), tr("write"));
        let s_act = Stat::new(tr("activeTime"), true);
        let s_resp = Stat::new(tr("response"), true);
        let s_read = Stat::new(tr("readSpeed"), true);
        let s_write = Stat::new(tr("writeSpeed"), true);
        let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let k_cap = Kv::new(tr("capacity"), &group);
        let k_type = Kv::new(tr("type"), &group);
        let k_sys = Kv::new(tr("systemDisk"), &group);
        let k_mnt = Kv::new(tr("mounts"), &group);
        let k_dev = Kv::new(tr("device"), &group);
        let root = self.detail_root();
        root.append(&head.root);
        root.append(&g1.root);
        root.append(&g2.root);
        root.append(&legend);
        root.append(&self.stats_block(&[vec![&s_act, &s_resp], vec![&s_read, &s_write]], &[&k_cap, &k_type, &k_sys, &k_mnt, &k_dev]));
        let dev = dev.clone();
        let (h, c1, c2) = (head.clone(), g1.clone(), g2.clone());
        let (sa, sr, srd, sw) = (s_act.clone(), s_resp.clone(), s_read.clone(), s_write.clone());
        let (kc, kty, ks, km, kd) = (k_cap.clone(), k_type.clone(), k_sys.clone(), k_mnt.clone(), k_dev.clone());
        let c2_max = Cell::new(0.0);
        let update = Box::new(move |page: &Perf, p: &Value| {
            let Some(d) = p["disks"].as_array().and_then(|a| a.iter().find(|x| util::text(&x["name"]) == dev.name)) else { return };
            h.set_title(&page.disk_title(&dev, d));
            h.set_right(&util::text(&d["model"]));
            let name = dev.name.clone();
            let tails = page.tails(|hist| match hist.disks.get(&name) {
                Some(x) => vec![&x.read, &x.write],
                None => vec![&EMPTY, &EMPTY],
            });
            let max = nice_max(max_of(&tails), 1_048_576.0);
            page.feed(&c1.chart, &c1.fed, false, |hist| vec![hist.disks.get(&name).map(|x| &x.active).unwrap_or(&EMPTY)]);
            if c2_max.get() != max {
                c2_max.set(max);
                c2.chart.set_max(max);
            }
            page.feed(&c2.chart, &c2.fed, true, |hist| match hist.disks.get(&name) {
                Some(x) => vec![&x.read, &x.write],
                None => vec![&EMPTY, &EMPTY],
            });
            c2.set_right(&trim_zero(rate(max)));
            sa.set(&format!("{}%", util::num(&d["activePct"]).round() as i64));
            srd.set(&rate(util::num(&d["readBps"])));
            sw.set(&rate(util::num(&d["writeBps"])));
            kc.set(&bytes(util::num(&d["capacity"])));
            kty.set(&kind_text(&util::text(&d["kind"])));
            let mounts: Vec<String> = d["mounts"].as_array().into_iter().flatten().map(util::text).collect();
            ks.set(tr(if mounts.iter().any(|m| m == "/") { "yes" } else { "no" }));
            km.set(&if mounts.is_empty() { DASH.to_owned() } else { mounts.join(", ") });
            let resp = d["responseMs"].as_f64().filter(|v| v.is_finite());
            sr.show(resp.is_some());
            if let Some(r) = resp {
                sr.set(&format!("{} ms", number_min(r, 1, 1)));
            }
            let device = util::text(&d["device"]);
            kd.show(!device.is_empty());
            if !device.is_empty() {
                kd.set(&format!("/dev/{device}"));
            }
        });
        Detail { root, head, stats: vec![s_act, s_resp, s_read, s_write], kvs: vec![k_cap, k_type, k_sys, k_mnt, k_dev], update }
    }

    fn net_detail(&self, dev: &Dev, color: &str) -> Detail {
        let head = Head::new("", "");
        let g = self.graph(tr("throughput"), "", color, true, 125_000.0, 0);
        let legend = self.legend(color, tr("receive"), tr("send"));
        let s_send = Stat::new(tr("send"), true);
        let s_recv = Stat::new(tr("receive"), true);
        let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let k_ad = Kv::new(tr("adapter"), &group);
        let k_if = Kv::new(tr("iface"), &group);
        let k_type = Kv::new(tr("connType"), &group);
        let k_link = Kv::new(tr("linkSpeed"), &group);
        let k4 = Kv::new(tr("ipv4"), &group);
        let k6 = Kv::new(tr("ipv6"), &group);
        let root = self.detail_root();
        root.append(&head.root);
        root.append(&g.root);
        root.append(&legend);
        root.append(&self.stats_block(&[vec![&s_send, &s_recv]], &[&k_ad, &k_if, &k_type, &k_link, &k4, &k6]));
        let iface = dev.iface.clone();
        let (h, gg, ss, sr) = (head.clone(), g.clone(), s_send.clone(), s_recv.clone());
        let (ka, ki, kt, kl, k4c, k6c) = (k_ad.clone(), k_if.clone(), k_type.clone(), k_link.clone(), k4.clone(), k6.clone());
        let chart_max = Cell::new(0.0);
        let update = Box::new(move |page: &Perf, p: &Value| {
            let Some(n) = p["network"].as_array().and_then(|a| a.iter().find(|x| util::text(&x["iface"]) == iface)) else { return };
            let name = [&n["name"], &n["iface"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
            h.set_title(&name);
            h.set_right(&util::text(&n["iface"]));
            let tails = page.tails(|hist| match hist.nets.get(&iface) {
                Some(x) => vec![&x.rx, &x.tx],
                None => vec![&EMPTY, &EMPTY],
            });
            let max = nice_max(max_of(&tails), 12500.0);
            if chart_max.get() != max {
                chart_max.set(max);
                gg.chart.set_max(max);
            }
            page.feed(&gg.chart, &gg.fed, true, |hist| match hist.nets.get(&iface) {
                Some(x) => vec![&x.rx, &x.tx],
                None => vec![&EMPTY, &EMPTY],
            });
            gg.set_right(&trim_zero(bitrate(max)));
            ss.set(&bitrate(util::num(&n["txBps"])));
            sr.set(&bitrate(util::num(&n["rxBps"])));
            let adapter = util::text(&n["adapter"]);
            ka.set(if adapter.is_empty() { &iface } else { &adapter });
            ki.show(!adapter.is_empty());
            ki.set(&iface);
            kt.set(&kind_text(&util::text(&n["kind"])));
            let link = util::num(&n["linkSpeedMbps"]);
            kl.show(link > 0.0);
            if link > 0.0 {
                kl.set(&if link >= 1000.0 { format!("{} Gbps", number(link / 1000.0, 1)) } else { format!("{} Mbps", number(link, 0)) });
            }
            let or_dash = |v: &Value| {
                let s = util::text(v);
                if s.is_empty() { DASH.to_owned() } else { s }
            };
            k4c.set(&or_dash(&n["ipv4"]));
            k6c.set(&or_dash(&n["ipv6"]));
        });
        Detail { root, head, stats: vec![s_send, s_recv], kvs: vec![k_ad, k_if, k_type, k_link, k4, k6], update }
    }

    fn gpu_detail(&self, dev: &Dev, color: &str) -> Detail {
        let head = Head::new(&fill(tr("gpu"), &[("n", &dev.index.to_string())]), "");
        let usage_over = fill(tr("usageOver"), &[("d", &dur_label(self.graph_seconds()))]);
        let g1 = self.graph(&usage_over, "100%", color, true, 100.0, 0);
        let g2 = self.graph(tr("gpuMem"), "", color, false, 100.0, 4);
        let s_use = Stat::new(tr("gpuUsage"), true);
        let s_mem = Stat::new(tr("gpuMemShort"), true);
        let s_temp = Stat::new(tr("gpuTemp"), true);
        let s_3d = Stat::new(tr("e3d"), false);
        let s_enc = Stat::new(tr("encode"), false);
        let s_dec = Stat::new(tr("decode"), false);
        let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let k_name = Kv::new(tr("name"), &group);
        let k_vendor = Kv::new(tr("vendor"), &group);
        let k_driver = Kv::new(tr("driver"), &group);
        let k_power = Kv::new(tr("power"), &group);
        let k_clock = Kv::new(tr("clock"), &group);
        let root = self.detail_root();
        root.append(&head.root);
        root.append(&g1.root);
        root.append(&g2.root);
        let stats = self.stats_block(&[vec![&s_use, &s_mem, &s_temp], vec![&s_3d, &s_enc, &s_dec]], &[&k_name, &k_vendor, &k_driver, &k_power, &k_clock]);
        root.append(&stats);
        // the engines row (3D, encode, decode) is the second row of the main stats
        let engines = s_3d.root.parent();
        let index = dev.index;
        let (h, c1, c2) = (head.clone(), g1.clone(), g2.clone());
        let (su, sm, st, s3, se, sd) = (s_use.clone(), s_mem.clone(), s_temp.clone(), s_3d.clone(), s_enc.clone(), s_dec.clone());
        let (kn, kv, kd, kp, kc) = (k_name.clone(), k_vendor.clone(), k_driver.clone(), k_power.clone(), k_clock.clone());
        let update = Box::new(move |page: &Perf, p: &Value| {
            let Some(g) = p["gpus"].as_array().and_then(|a| a.get(index)) else { return };
            h.set_right(&util::text(&g["name"]));
            page.feed(&c1.chart, &c1.fed, false, |hist| vec![hist.gpus.get(&index).map(|x| &x.usage).unwrap_or(&EMPTY)]);
            page.feed(&c2.chart, &c2.fed, false, |hist| vec![hist.gpus.get(&index).map(|x| &x.mem).unwrap_or(&EMPTY)]);
            let mem_total = util::num(&g["memTotal"]);
            let b1 = |v: f64| kit::format::bytes_digits(v, Some(1));
            c2.set_right(&if mem_total > 0.0 { b1(mem_total) } else { String::new() });
            su.set(&g["usage"].as_f64().map(|u| format!("{}%", u.round() as i64)).unwrap_or_else(|| tr("notAvailable").into()));
            sm.set(&if mem_total > 0.0 { format!("{}/{}", b1(util::num(&g["memUsed"])), b1(mem_total)) } else { tr("notAvailable").into() });
            st.set(&temp(&g["temperature"]).unwrap_or_else(|| tr("notAvailable").into()));
            let name = util::text(&g["name"]);
            kn.set(if name.is_empty() { tr("unknown") } else { &name });
            let vendor = util::text(&g["vendor"]);
            kv.set(if vendor.is_empty() {
                tr("unknown")
            } else if vendor.eq_ignore_ascii_case("nvidia") {
                "NVIDIA"
            } else {
                &vendor
            });
            let e = g["engines"].as_object();
            if let Some(row) = &engines {
                row.set_visible(e.is_some());
            }
            for s in [&s3, &se, &sd] {
                s.show(e.is_some());
            }
            if let Some(e) = e {
                let pct = |k: &str| e.get(k).and_then(Value::as_f64).map(|v| format!("{}%", v.round() as i64)).unwrap_or_else(|| DASH.into());
                s3.set(&pct("3d"));
                se.set(&pct("encode"));
                sd.set(&pct("decode"));
            }
            let driver = util::text(&g["driver"]);
            kd.show(!driver.is_empty());
            if !driver.is_empty() {
                kd.set(&driver);
            }
            let power = g["power"].as_f64().filter(|v| v.is_finite());
            kp.show(power.is_some());
            if let Some(w) = power {
                kp.set(&format!("{} W", number(w, 0)));
            }
            let clock = util::num(&g["clockMHz"]);
            kc.show(clock > 0.0);
            if clock > 0.0 {
                kc.set(&format!("{} MHz", number(clock, 0)));
            }
        });
        Detail { root, head, stats: vec![s_use, s_mem, s_temp, s_3d, s_enc, s_dec], kvs: vec![k_name, k_vendor, k_driver, k_power, k_clock], update }
    }

    // ------------------------------------------------------------------------------------------ commands

    fn view_menu(&self, anchor: &gtk::Button) {
        let mut items = Vec::new();
        if *self.selected.borrow() == "cpu" {
            let mode = self.cpu_mode.borrow().clone();
            items.push(MenuItem::header(tr("graph")));
            for (m, key) in [("overall", "overall"), ("perCore", "perCore")] {
                let w = self.me.clone();
                items.push(
                    MenuItem::new(tr(key), move || {
                        if let Some(p) = w.upgrade() {
                            p.set_cpu_mode(m);
                        }
                    })
                    .radio(mode == m)
                    .name(&format!("taskmgr-menu-cpu-{m}")),
                );
            }
            items.push(MenuItem::separator());
        }
        let cur = self.graph_seconds();
        let durations: Vec<MenuItem> = DURATIONS
            .iter()
            .map(|&s| {
                let a = self.app.clone();
                MenuItem::new(&dur_label(s), move || a.set_setting("taskmgr.graphSeconds", json!(s), || {}))
                    .radio(cur == s)
                    .name(&format!("taskmgr-menu-duration-{s}"))
            })
            .collect();
        items.push(MenuItem::submenu(tr("duration"), durations).glyph("clock").name("taskmgr-menu-duration"));
        kit::controls::show_menu_for(anchor, items);
    }

    fn set_cpu_mode(&self, m: &str) {
        *self.cpu_mode.borrow_mut() = m.to_owned();
        util::save("perf.cpuMode", json!(m));
        self.build_detail();
    }

    fn copy_summary(&self) {
        let Some(d) = self.detail.borrow().clone() else { return };
        let text = d.lines().into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
        layout::copy_text(&self.root, &text);
        self.app.toast_success(tr("copied"), 2500);
    }

    // ---------------------------------------------------------------------------------------------- data

    fn on_data(&self, p: &Rc<Value>) {
        if !p["cpu"].is_object() {
            return;
        }
        let first = self.perf.borrow().is_none();
        *self.perf.borrow_mut() = Some(p.clone());
        self.build_list(p);
        self.update_list(p);
        let detail = self.detail.borrow().clone();
        match detail {
            Some(d) if !first => (d.update)(self, p),
            _ => self.build_detail(),
        }
    }

    fn on_volumes(&self, vols: &Value) {
        let map = vols.as_array().into_iter().flatten().map(|v| (util::text(&v["mount"]), v.clone())).collect();
        *self.vol_by_mount.borrow_mut() = Some(map);
        let Some(perf) = self.perf.borrow().clone() else { return };
        self.update_list(&perf);
        if let Some(d) = self.detail.borrow().clone() {
            (d.update)(self, &perf);
        }
    }
}

impl Page for Perf {
    fn id(&self) -> &'static str {
        "performance"
    }

    fn title(&self) -> String {
        tr("title").to_owned()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn commands(&self) -> Vec<gtk::Widget> {
        self.commands.clone()
    }

    fn fill(&self) -> bool {
        true
    }

    fn mount(&self, params: &Value) {
        let device = util::text(&params["device"]);
        if !device.is_empty() {
            *self.selected.borrow_mut() = device.clone();
            util::save("perf.device", json!(device));
        }
        let w = self.me.clone();
        let perf = self.app.store.subscribe("performance", move |d| {
            if let Some(p) = w.upgrade() {
                p.on_data(d);
            }
        });
        let w = self.me.clone();
        let w2 = self.me.clone();
        let vols = self.app.store.subscribe_with_error(
            "volumes",
            move |d| {
                if let Some(p) = w.upgrade() {
                    p.on_volumes(d);
                }
            },
            move |_| {
                if let Some(p) = w2.upgrade() {
                    p.vols_failed.set(true);
                    if let Some(perf) = p.perf.borrow().clone() {
                        p.update_list(&perf);
                    }
                }
            },
        );
        self.subs.borrow_mut().extend([perf, vols]);
        if !device.is_empty() && self.perf.borrow().is_some() {
            self.select(&device, false);
        }
    }

    fn navigate(&self, params: &Value) {
        let device = util::text(&params["device"]);
        if !device.is_empty() {
            self.select(&device, false);
        }
    }

    fn unmount(&self) {
        self.subs.borrow_mut().clear();
    }

    fn refresh(&self) -> bool {
        self.app.store.refresh("performance");
        true
    }
}
