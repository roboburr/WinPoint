//! Task Manager's shared helpers (js/apps/taskmgr/util.js): Task Manager style number formatting, the graph and
//! storage colours, the app's own glyphs (efficiency leaf, end task, stop, pause, the app tile, the health
//! shield) as symbolic paintables, the per-viewer conveniences the WebKit app kept in localStorage (a JSON file
//! next to settings.json), tooltip wrappers for disabled buttons, command bar buttons and text templates.
use kit::format::number_min;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Duration;

// ------------------------------------------------------------------------------------------- persistence

thread_local! {
    static STATE: RefCell<Option<Value>> = const { RefCell::new(None) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// The file the WebKit app's `localStorage` keys live in now ("win11shell.taskmgr.<key>"): navCompact,
/// proc.grouped, table.<id>, effNoConfirm, restartNoConfirm, perf.*, storage.*.
fn state_path() -> PathBuf {
    kit::ctx::settings_path().with_file_name("native-taskmgr.json")
}

fn with_state<R>(f: impl FnOnce(&mut Value) -> R) -> R {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_none() {
            let v = std::fs::read(state_path())
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            *s = Some(v);
        }
        match s.as_mut() {
            Some(v) => f(v),
            None => f(&mut json!({})),
        }
    })
}

/// A remembered value (Null when there is none).
pub fn load(key: &str) -> Value {
    with_state(|s| s.get(key).cloned().unwrap_or(Value::Null))
}

/// A remembered flag, `fallback` when there is none.
pub fn load_bool(key: &str, fallback: bool) -> bool {
    load(key).as_bool().unwrap_or(fallback)
}

/// Remember a value (written to disk shortly after, together with other changes).
pub fn save(key: &str, value: Value) {
    with_state(|s| {
        if let Some(o) = s.as_object_mut() {
            o.insert(key.to_owned(), value);
        }
    });
    if SAVE_PENDING.with(|p| p.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(Duration::from_millis(400), || {
        SAVE_PENDING.with(|p| p.set(false));
        flush();
    });
}

/// Write the remembered values now.
pub fn flush() {
    let text = with_state(|s| serde_json::to_vec_pretty(s).unwrap_or_default());
    let path = state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

// -------------------------------------------------------------------------------------------- formatting

const DASH: &str = "\u{2013}";

/// cpu(3.456) -> "3,5%"; cpu(0.04) -> "0%" (Task Manager style).
pub fn cpu_pct(v: f64) -> String {
    if !v.is_finite() {
        return DASH.into();
    }
    if v < 0.05 {
        return "0%".into();
    }
    format!("{}%", number_min(v, 1, 1))
}

/// Memory in MB with one decimal, like Task Manager ("245,3 MB"); from 10 GB on in GB.
pub fn mem_mb(n: f64) -> String {
    if !n.is_finite() {
        return DASH.into();
    }
    let mb = n / 1_048_576.0;
    if mb >= 10_240.0 {
        return format!("{} GB", number_min(mb / 1024.0, 1, 1));
    }
    format!("{} MB", number_min(mb, 1, 1))
}

/// Disk rate in MB/s, Task Manager style ("0 MB/s", "0,1 MB/s", "12,4 MB/s").
pub fn disk_mbs(n: f64) -> String {
    if !n.is_finite() {
        return DASH.into();
    }
    let mb = n / 1_048_576.0;
    if mb < 0.05 {
        return "0 MB/s".into();
    }
    format!("{} MB/s", number_min(mb, 1, 1))
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// How full a drive is (0..100): the host's df-style `percent` (used / (used + free)), as Verkenner, Instellingen
/// and the health check show it.
pub fn vol_pct(v: &Value) -> f64 {
    if !v.is_object() {
        return 0.0;
    }
    let p = match v["percent"].as_f64().filter(|p| p.is_finite()) {
        Some(p) => p,
        None => {
            let used = num(&v["used"]);
            let free = num(&v["free"]);
            if used + free > 0.0 { used / (used + free) * 100.0 } else { 0.0 }
        }
    };
    p.clamp(0.0, 100.0)
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// MHz as "3,40 GHz" ("–" without a value).
pub fn ghz(mhz: f64) -> String {
    if mhz.is_finite() && mhz != 0.0 { format!("{} GHz", number_min(mhz / 1000.0, 2, 2)) } else { DASH.into() }
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// A whole percentage ("23%", "–").
pub fn pct_int(v: f64) -> String {
    if v.is_finite() { format!("{}%", v.round() as i64) } else { DASH.into() }
}

/// A JSON number as f64 (0 when missing).
pub fn num(v: &Value) -> f64 {
    v.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0)
}

/// A JSON value as text ("" for null; numbers plainly).
pub fn text(v: &Value) -> String {
    kit::table::value_text(v)
}

/// "{name} is gestart" with the named values filled in.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (k, v) in values {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// i18n.js plurals: `one` for exactly 1, else `other`, with {n} filled in.
pub fn plural(n: i64, one: &str, other: &str) -> String {
    fill(if n == 1 { one } else { other }, &[("n", &n.to_string())])
}

// ------------------------------------------------------------------------------------------------ colours

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// The graph colour of a device kind (a token for kit::chart): the kind's own colour while colour-keyed graphs
/// are on (taskmgr.colorKeyedGraphs), else the accent.
pub fn kind_color(kind: &str) -> &'static str {
    let keyed = kit::try_ctx().map(|c| c.setting("taskmgr.colorKeyedGraphs").as_bool().unwrap_or(true)).unwrap_or(true);
    if !keyed {
        return "accent";
    }
    match kind {
        "cpu" => kit::chart::colors::CPU,
        "memory" => kit::chart::colors::MEMORY,
        "disk" => kit::chart::colors::DISK,
        "network" => kit::chart::colors::NETWORK,
        "gpu" => kit::chart::colors::GPU,
        _ => "accent",
    }
}

/// Task Manager's own colour tokens (css/taskmgr.css `--tm-*`) for the theme in effect, as "#rrggbb" (cairo
/// drawing cannot read app CSS variables). None for an unknown name.
pub fn app_color(name: &str) -> Option<&'static str> {
    let dark = kit::theme::dark();
    let name = name.trim_start_matches("--").trim_start_matches("tm-");
    Some(match (name, dark) {
        ("leaf", false) => "#3c9a2e",
        ("leaf", true) => "#6ccb5f",
        ("leaf-vein", false) => "#e8f7e3",
        ("leaf-vein", true) => "#1d3d17",
        ("hang", false) => "#b0490c",
        ("hang", true) => "#fcb36b",
        ("cat-browser", false) => "#d9532a",
        ("cat-browser", true) => "#ff8a5c",
        ("cat-chat", false) => "#5b5fc7",
        ("cat-chat", true) => "#8f93ff",
        ("cat-appdata", false) => "#8e5ccb",
        ("cat-appdata", true) => "#c49bff",
        ("cat-snaps", false) => "#e2681f",
        ("cat-snaps", true) => "#ff9a52",
        ("cat-flatpak", false) => "#2f7fd1",
        ("cat-flatpak", true) => "#5eaaff",
        ("cat-cache", false) => "#8d969d",
        ("cat-cache", true) => "#9aa3aa",
        ("cat-other", false) => "#b6bcc2",
        ("cat-other", true) => "#6b7379",
        ("danger-bar", false) => "#c42b1c",
        ("danger-bar", true) => "#ff6b6b",
        ("warn-bar", false) => "#d1880c",
        ("warn-bar", true) => "#f2b33d",
        _ => return None,
    })
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// A colour name for kit::chart: an app token ("tm-cat-chat") becomes its hex, a kit token stays as it is.
pub fn chart_color(name: &str) -> String {
    match app_color(name) {
        Some(hex) => hex.to_owned(),
        None => name.trim_start_matches("--").to_owned(),
    }
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// The colour of a storage category (util.js CATEGORY_COLORS), resolved for kit::chart.
pub fn category_color(category: &str) -> String {
    chart_color(match category {
        "documents" => "chart-1",
        "pictures" => "chart-3",
        "videos" => "chart-2",
        "music" => "chart-4",
        "downloads" => "chart-5",
        "desktop" => "chart-7",
        "trash" => "chart-8",
        "browser" => "tm-cat-browser",
        "chat" => "tm-cat-chat",
        "dev" => "chart-6",
        "appdata" => "tm-cat-appdata",
        "snaps" => "tm-cat-snaps",
        "flatpak" => "tm-cat-flatpak",
        "cache" => "tm-cat-cache",
        _ => "tm-cat-other",
    })
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// The sunburst / treemap palette for top-level children (util.js RING_COLORS).
pub const RING_COLORS: [&str; 10] =
    ["chart-1", "chart-2", "chart-3", "chart-5", "chart-4", "chart-6", "chart-7", "tm-cat-chat", "tm-cat-browser", "tm-cat-flatpak"];

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// RING_COLORS[i % 10] resolved for kit::chart.
pub fn ring_color(i: usize) -> String {
    chart_color(RING_COLORS[i % RING_COLORS.len()])
}

/// 0..1 -> heat bucket 0..10 (Win11's pale yellow to orange cells).
pub fn heat_bucket(f: f64) -> u8 {
    if !f.is_finite() {
        return 0;
    }
    (f.clamp(0.0, 1.0).powf(0.75) * 10.0).round().clamp(0.0, 10.0) as u8
}

// ------------------------------------------------------------------------------------------------- glyphs

/// Task Manager's own glyphs (util.js leafIcon, leafOutline, endTaskIcon, stopIcon, pauseIcon, statusGlyph and
/// main.js appGlyph). The outline ones take the widget's colour like the kit's glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmGlyph {
    /// The green efficiency-mode leaf (filled).
    Leaf,
    /// The leaf outline (buttons, menus).
    LeafOutline,
    /// Circle with a slash (End task).
    EndTask,
    /// Rounded square (Stop).
    Stop,
    /// Two bars (Suspended).
    Pause,
    /// The app icon: a green tile with a pulse line.
    AppTile,
    /// The health shield with a check (ok) or an exclamation mark (warning / critical).
    StatusOk,
    StatusWarning,
    StatusCritical,
}

impl TmGlyph {
    fn tinted(self) -> bool {
        matches!(self, TmGlyph::LeafOutline | TmGlyph::EndTask | TmGlyph::Stop | TmGlyph::Pause)
    }

    fn view(self) -> f64 {
        match self {
            TmGlyph::AppTile => 16.0,
            TmGlyph::StatusOk | TmGlyph::StatusWarning | TmGlyph::StatusCritical => 48.0,
            _ => 20.0,
        }
    }
}

const LEAF: &str = "M16.5 3.5c-6.6-.4-11.5 2.3-12.3 7.4-.3 1.8.3 3.4 1.3 4.5 1.1 1 2.7 1.5 4.4 1.2 5-.9 7-5.8 6.6-13.1Z";
const SHIELD: &str = "M24 4.5 39 9.8c.9.3 1.5 1.2 1.5 2.2v9.7c0 9.6-6.4 18.2-15.8 21.6-.5.2-1 .2-1.4 0C14 39.9 7.5 31.3 7.5 21.7V12c0-1 .6-1.9 1.5-2.2Z";

/// Draw a glyph into a `side` x `side` square at the origin of `cr` in `color` (for custom drawing).
pub fn draw_tm_glyph(cr: &gtk::cairo::Context, g: TmGlyph, side: f64, color: &gdk::RGBA) {
    let s = side / g.view();
    cr.save().ok();
    cr.scale(s, s);
    let (r, gg, b, a) = (color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let hex = |name: &str| kit::theme::rgba(app_color(name).unwrap_or("#000000"));
    let set = |c: &gdk::RGBA| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
    match g {
        TmGlyph::Leaf => {
            path(cr, LEAF);
            set(&hex("leaf"));
            cr.fill().ok();
            path(cr, "M4.2 16.6c2.2-3.6 4.8-6.2 8.4-8.1");
            set(&hex("leaf-vein"));
            cr.set_line_width(1.3);
            cr.stroke().ok();
        }
        TmGlyph::LeafOutline => {
            cr.set_source_rgba(r, gg, b, a);
            cr.set_line_width(kit::icons::STROKE);
            path(cr, LEAF);
            cr.stroke().ok();
            path(cr, "M3.6 17.2c2.2-3.8 5-6.6 8.9-8.7");
            cr.stroke().ok();
        }
        TmGlyph::EndTask => {
            cr.set_source_rgba(r, gg, b, a);
            cr.set_line_width(kit::icons::STROKE);
            cr.new_sub_path();
            cr.arc(10.0, 10.0, 7.25, 0.0, std::f64::consts::TAU);
            cr.stroke().ok();
            path(cr, "M5 15 15 5");
            cr.stroke().ok();
        }
        TmGlyph::Stop => {
            cr.set_source_rgba(r, gg, b, a);
            cr.set_line_width(kit::icons::STROKE);
            rounded(cr, 5.0, 5.0, 10.0, 10.0, 1.6);
            cr.stroke().ok();
        }
        TmGlyph::Pause => {
            cr.set_source_rgba(r, gg, b, a);
            cr.set_line_width(1.6);
            path(cr, "M7.25 5v10M12.75 5v10");
            cr.stroke().ok();
        }
        TmGlyph::AppTile => {
            rounded(cr, 0.5, 1.5, 15.0, 13.0, 3.0);
            let grad = gtk::cairo::LinearGradient::new(0.5, 1.5, 15.5, 14.5);
            grad.add_color_stop_rgb(0.0, 0x38 as f64 / 255.0, 0xd3 as f64 / 255.0, 0x9f as f64 / 255.0);
            grad.add_color_stop_rgb(1.0, 0x0f as f64 / 255.0, 0x8a as f64 / 255.0, 0x6a as f64 / 255.0);
            if cr.set_source(&grad).is_ok() {
                cr.fill().ok();
            }
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.35);
            path(cr, "M2.6 9.4h2.3l1.4-3.3 2 5.4 1.6-3.6.8 1.5h2.7");
            cr.stroke().ok();
        }
        TmGlyph::StatusOk | TmGlyph::StatusWarning | TmGlyph::StatusCritical => {
            let (top, bottom) = match g {
                TmGlyph::StatusOk => ("#3fbf62", "#0f7b3a"),
                TmGlyph::StatusWarning => ("#ffb23f", "#d56a00"),
                _ => ("#ff6b5e", "#b3261e"),
            };
            let (t, bt) = (kit::theme::rgba(top), kit::theme::rgba(bottom));
            path(cr, SHIELD);
            let grad = gtk::cairo::LinearGradient::new(0.0, 4.5, 0.0, 43.5);
            grad.add_color_stop_rgb(0.0, t.red() as f64, t.green() as f64, t.blue() as f64);
            grad.add_color_stop_rgb(1.0, bt.red() as f64, bt.green() as f64, bt.blue() as f64);
            if cr.set_source(&grad).is_ok() {
                cr.fill_preserve().ok();
            }
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
            cr.set_line_width(1.0);
            cr.stroke().ok();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            if g == TmGlyph::StatusOk {
                cr.set_line_width(3.2);
                path(cr, "M16.5 24.2 21.6 29.3 31.8 18.6");
                cr.stroke().ok();
            } else {
                cr.set_line_width(3.4);
                path(cr, "M24 15.5v10.5");
                cr.stroke().ok();
                cr.new_sub_path();
                cr.arc(24.0, 32.2, 2.1, 0.0, std::f64::consts::TAU);
                cr.fill().ok();
            }
        }
    }
    cr.restore().ok();
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    cr.close_path();
}

/// An SVG path's `d` (M L H V C S Z, absolute and relative, implicit repeats) onto cairo.
pub fn path(cr: &gtk::cairo::Context, d: &str) {
    struct Pen<'a> {
        cr: &'a gtk::cairo::Context,
        x: f64,
        y: f64,
        sx: f64,
        sy: f64,
        ctrl: Option<(f64, f64)>,
    }
    impl Pen<'_> {
        fn run(&mut self, cmd: char, n: &[f64]) {
            let rel = cmd.is_ascii_lowercase();
            let up = cmd.to_ascii_uppercase();
            if up == 'Z' {
                self.cr.close_path();
                self.x = self.sx;
                self.y = self.sy;
                self.ctrl = None;
                return;
            }
            let mut k = 0;
            loop {
                let (bx, by) = (self.x, self.y);
                let ax = |v: f64| if rel { bx + v } else { v };
                let ay = |v: f64| if rel { by + v } else { v };
                match up {
                    'M' | 'L' if k + 1 < n.len() => {
                        let (nx, ny) = (ax(n[k]), ay(n[k + 1]));
                        if up == 'M' && k == 0 {
                            self.cr.move_to(nx, ny);
                            self.sx = nx;
                            self.sy = ny;
                        } else {
                            self.cr.line_to(nx, ny);
                        }
                        (self.x, self.y, self.ctrl) = (nx, ny, None);
                        k += 2;
                    }
                    'H' if k < n.len() => {
                        self.x = ax(n[k]);
                        self.cr.line_to(self.x, self.y);
                        self.ctrl = None;
                        k += 1;
                    }
                    'V' if k < n.len() => {
                        self.y = ay(n[k]);
                        self.cr.line_to(self.x, self.y);
                        self.ctrl = None;
                        k += 1;
                    }
                    'C' if k + 5 < n.len() => {
                        let (x1, y1, x2, y2) = (ax(n[k]), ay(n[k + 1]), ax(n[k + 2]), ay(n[k + 3]));
                        let (x3, y3) = (ax(n[k + 4]), ay(n[k + 5]));
                        self.cr.curve_to(x1, y1, x2, y2, x3, y3);
                        (self.x, self.y, self.ctrl) = (x3, y3, Some((x2, y2)));
                        k += 6;
                    }
                    'S' if k + 3 < n.len() => {
                        let (x1, y1) = match self.ctrl {
                            Some((cx, cy)) => (2.0 * bx - cx, 2.0 * by - cy),
                            None => (bx, by),
                        };
                        let (x2, y2, x3, y3) = (ax(n[k]), ay(n[k + 1]), ax(n[k + 2]), ay(n[k + 3]));
                        self.cr.curve_to(x1, y1, x2, y2, x3, y3);
                        (self.x, self.y, self.ctrl) = (x3, y3, Some((x2, y2)));
                        k += 4;
                    }
                    _ => break,
                }
            }
        }
    }
    let mut pen = Pen { cr, x: 0.0, y: 0.0, sx: 0.0, sy: 0.0, ctrl: None };
    cr.new_path();
    let chars: Vec<char> = d.chars().collect();
    let mut nums: Vec<f64> = Vec::new();
    let mut cmd: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_alphabetic() {
            if let Some(prev) = cmd {
                pen.run(prev, &nums);
            }
            nums.clear();
            cmd = Some(c);
            i += 1;
            continue;
        }
        if c == '-' || c == '.' || c.is_ascii_digit() {
            let start = i;
            let mut dot = c == '.';
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !dot {
                    dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            let s: String = chars[start..i].iter().collect();
            if let Ok(v) = s.parse::<f64>() {
                nums.push(v);
            }
            continue;
        }
        i += 1;
    }
    if let Some(prev) = cmd {
        pen.run(prev, &nums);
    }
}

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    pub struct Paint {
        pub glyph: Cell<TmGlyph>,
        pub size: Cell<i32>,
        pub owner: glib::WeakRef<gtk::Widget>,
    }

    impl Default for Paint {
        fn default() -> Paint {
            Paint { glyph: Cell::new(TmGlyph::EndTask), size: Cell::new(16), owner: glib::WeakRef::new() }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Paint {
        const NAME: &'static str = "TaskmgrGlyph";
        type Type = super::Paint;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for Paint {}

    impl PaintableImpl for Paint {
        fn intrinsic_width(&self) -> i32 {
            self.size.get()
        }
        fn intrinsic_height(&self) -> i32 {
            self.size.get()
        }
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let color = self.owner.upgrade().map(|w| w.color()).unwrap_or(gdk::RGBA::WHITE);
            self.obj().draw(snapshot, width, height, &color);
        }
    }

    impl SymbolicPaintableImpl for Paint {
        fn snapshot_symbolic(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            let color = colors
                .first()
                .copied()
                .or_else(|| self.owner.upgrade().map(|w| w.color()))
                .unwrap_or(gdk::RGBA::WHITE);
            self.obj().draw(snapshot, width, height, &color);
        }

        // GTK 4.22 calls snapshot_with_weight; the default chains to an empty parent slot (see the kit's Glyph).
        fn snapshot_with_weight(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA], _weight: f64) {
            self.snapshot_symbolic(snapshot, width, height, colors);
        }
    }
}

glib::wrapper! {
    /// A Task Manager glyph as a paintable (see `icon`).
    pub struct Paint(ObjectSubclass<imp::Paint>) @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl Paint {
    pub fn new(g: TmGlyph, px: i32, owner: &impl IsA<gtk::Widget>) -> Paint {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let p: Paint = glib::Object::new();
        p.imp().glyph.set(g);
        p.imp().size.set(px.max(1));
        p.imp().owner.set(Some(owner.upcast_ref()));
        p
    }

    fn draw(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, color: &gdk::RGBA) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else { return };
        let side = width.min(height);
        if side <= 0.0 {
            return;
        }
        let g = self.imp().glyph.get();
        let rect = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let cr = snapshot.append_cairo(&rect);
        cr.translate((width - side) / 2.0, (height - side) / 2.0);
        let color = if g.tinted() { *color } else { gdk::RGBA::WHITE };
        draw_tm_glyph(&cr, g, side, &color);
    }
}

/// A Task Manager glyph as an image, `px` square (class w11-glyph like the kit's, so it takes the CSS colour).
pub fn icon(g: TmGlyph, px: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_paintable(Some(&Paint::new(g, px, &image)));
    image.set_pixel_size(px);
    image.add_css_class("w11-glyph");
    image.add_css_class("taskmgr-glyph");
    if g == TmGlyph::Leaf {
        image.add_css_class("taskmgr-leaf");
    }
    image.set_can_target(false);
    if !g.tinted() {
        // the fixed colours follow the theme when redrawn
        kit::theme::track(&image);
    }
    image
}

/// Show a Task Manager glyph in the first kit glyph image under `root` (a kit button or menu item made with a
/// placeholder glyph).
pub fn swap_glyph(root: &gtk::Widget, g: TmGlyph) {
    fn find(w: &gtk::Widget) -> Option<gtk::Image> {
        if let Some(i) = w.downcast_ref::<gtk::Image>()
            && i.has_css_class("w11-glyph")
        {
            return Some(i.clone());
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if !child.is::<gtk::Popover>()
                && let Some(i) = find(&child)
            {
                return Some(i);
            }
            c = child.next_sibling();
        }
        None
    }
    if let Some(img) = find(root) {
        let px = if img.pixel_size() > 0 { img.pixel_size() } else { 16 };
        img.set_paintable(Some(&Paint::new(g, px, &img)));
    }
}

/// After a kit menu opened on `anchor` (a context menu, a drop-down button), give the items named in `names`
/// their Task Manager glyph (the kit's menu items take glyph names only). Runs before the menu is drawn.
pub fn fix_menu_glyphs(anchor: &impl IsA<gtk::Widget>, names: Vec<(&'static str, TmGlyph)>) {
    let anchor = anchor.as_ref().downgrade();
    glib::idle_add_local_full(glib::Priority::HIGH, move || {
        if let Some(a) = anchor.upgrade() {
            let mut c = a.first_child();
            while let Some(child) = c {
                if child.is::<gtk::Popover>() && child.has_css_class("w11-menu") {
                    for (name, g) in &names {
                        if let Some(b) = find_named(&child, name) {
                            swap_glyph(&b, *g);
                        }
                    }
                }
                c = child.next_sibling();
            }
        }
        glib::ControlFlow::Break
    });
}

/// The first widget named `name` under `root` (also inside popovers).
pub fn find_named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut c = root.first_child();
    while let Some(child) = c {
        if let Some(w) = find_named(&child, name) {
            return Some(w);
        }
        c = child.next_sibling();
    }
    None
}

// ---------------------------------------------------------------------------------------- small widgets

/// An icon for a button or a card: a kit glyph name or a Task Manager glyph.
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Glyph(&'static str),
    Tm(TmGlyph),
}

impl Icon {
    pub fn image(self, px: i32) -> Option<gtk::Image> {
        match self {
            Icon::Glyph(name) => Some(kit::icons::glyph(name, px)),
            Icon::Tm(g) => Some(icon(g, px)),
        }
    }
}

/// A command bar button (`.tm-cmdbar .fl-btn--subtle`): a subtle button with a 16 px icon and its label
/// (class taskmgr-cmd-label; the label hides when the page is narrow).
pub fn cmd_button(icon: Icon, label: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-button");
    b.add_css_class("w11-subtle");
    b.add_css_class("taskmgr-cmd");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    if let Some(img) = icon.image(16) {
        img.add_css_class("taskmgr-cmd-icon");
        row.append(&img);
    }
    let l = gtk::Label::new(Some(label));
    l.add_css_class("taskmgr-cmd-label");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&l);
    b.set_child(Some(&row));
    b.set_tooltip_text(None);
    b
}

/// Relabel a button made by `cmd_button`.
pub fn set_button_label(b: &gtk::Button, label: &str) {
    if let Some(l) = first_label(b.upcast_ref()) {
        l.set_label(label);
    }
}

/// The label of a button made by `cmd_button` (for the narrow command bar).
pub fn first_label(w: &gtk::Widget) -> Option<gtk::Label> {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        return Some(l.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(l) = first_label(&child) {
            return Some(l);
        }
        c = child.next_sibling();
    }
    None
}

/// The divider between groups of command bar buttons (`.tm-cmd-sep`).
pub fn cmd_sep() -> gtk::Widget {
    let s = gtk::Box::new(gtk::Orientation::Vertical, 0);
    s.add_css_class("taskmgr-cmd-sep");
    s.set_size_request(1, 20);
    s.set_valign(gtk::Align::Center);
    s.upcast()
}

/// Wrap a widget so its tooltip also shows while it is disabled (util.js tipWrap).
pub fn tip_wrap(w: &impl IsA<gtk::Widget>, tip: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("taskmgr-tipwrap");
    b.set_valign(gtk::Align::Center);
    b.append(w.as_ref());
    set_tip(&b, tip);
    b
}

/// Set (or with "" remove) a wrapper's tooltip.
pub fn set_tip(w: &impl IsA<gtk::Widget>, tip: &str) {
    w.as_ref().set_tooltip_text(if tip.is_empty() { None } else { Some(tip) });
}

/// A label with the kit's defaults and extra classes.
pub fn label(text: &str, class: &str) -> gtk::Label {
    kit::controls::label(text, class, false)
}

/// A wrapping label.
pub fn wrap_label(text: &str, class: &str) -> gtk::Label {
    kit::controls::label(text, class, true)
}

/// Toggle a CSS class.
pub fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.as_ref().add_css_class(class);
    } else {
        w.as_ref().remove_css_class(class);
    }
}

/// An app icon or, without one, a glyph (`.tm-glyph`), `px` square.
pub fn app_or_glyph(icon_name: &str, glyph: &str, px: i32) -> gtk::Widget {
    if icon_name.is_empty() {
        let g = kit::icons::glyph(glyph, px);
        g.add_css_class("taskmgr-glyph-dim");
        g.upcast()
    } else {
        kit::icons::app_icon(icon_name, px).upcast()
    }
}

// ------------------------------------------------------------------------------------------------- paths

/// The executable path of a command line ("" when it is not absolute).
pub fn exe_path(cmd: &str) -> String {
    let s = cmd.trim();
    if !s.starts_with('/') {
        return String::new();
    }
    s.split(' ').next().unwrap_or("").to_owned()
}

/// The folder of a path ("/" for a top-level one).
pub fn dir_name(path: &str) -> String {
    match path.rfind('/') {
        Some(i) if i > 0 => path[..i].to_owned(),
        _ => "/".into(),
    }
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// The last part of a path.
pub fn base_name(path: &str) -> String {
    let p = path.trim_end_matches('/');
    let b = p.rsplit('/').next().unwrap_or("");
    if b.is_empty() { "/".into() } else { b.to_owned() }
}

#[allow(dead_code)] // page API for Prestaties / Samenvatting / Opslag (pages/mod.rs)
/// /home/robin/... -> ~/...
pub fn pretty_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("/home/") {
        return match rest.find('/') {
            Some(i) => format!("~{}", &rest[i..]),
            None => "~".into(),
        };
    }
    path.to_owned()
}
