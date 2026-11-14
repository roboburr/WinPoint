//! The sunburst chart of Opslag > Verkennen (js/apps/taskmgr/storage/sunburst.js), drawn with cairo: up to three
//! rings below the current folder, each top-level child its own hue (util::ring_color), deeper levels lighter
//! tints of it, "Overige" neutral. Pointing at an arc highlights it and its ancestors and shows a tooltip; a
//! click on a folder drills in, on a file picks it, on the centre goes one level up. Also the scan tree (`Node`)
//! the explorer, the side list and the volume bars share.
use super::layout::{self, DISPLAY_FONT};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::{PI, TAU};
use std::rc::Rc;

/// The rings below the current folder.
const RINGS: usize = 3;
/// The viewBox of sunburst.js.
const VB: f64 = 440.0;
const MIN_SWEEP: f64 = 0.006;

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "fileOne" => ("{n} bestand", "{n} file"),
        "fileOther" => ("{n} bestanden", "{n} files"),
        "ofParent" => ("{p} van {name}", "{p} of {name}"),
        "up" => ("Klik om een niveau omhoog te gaan", "Click to go up one level"),
        "other" => ("Overige", "Other"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

/// "1 bestand" / "12.345 bestanden".
pub fn files_text(n: f64) -> String {
    let count = if n.is_finite() { n.round() as i64 } else { 0 };
    crate::util::fill(tr(if count == 1 { "fileOne" } else { "fileOther" }), &[("n", &kit::format::number(n, 0))])
}

// ------------------------------------------------------------------------------------------------ the tree

/// One folder or file of a scan (storage.scan's tree): {name, path, size, files, isDir, children, other, count,
/// truncated}. Folders deeper than the first scan get their children grafted in later.
pub struct Node {
    pub name: String,
    pub path: String,
    pub size: f64,
    pub files: Cell<Option<f64>>,
    pub is_dir: bool,
    /// "Overige (n kleinere items)".
    pub other: bool,
    pub count: f64,
    pub truncated: Cell<bool>,
    pub children: RefCell<Vec<Rc<Node>>>,
}

impl Node {
    pub fn from_value(v: &Value) -> Rc<Node> {
        let children = v["children"].as_array().map(|a| a.iter().filter(|c| c.is_object()).map(Node::from_value).collect()).unwrap_or_default();
        Rc::new(Node {
            name: crate::util::text(&v["name"]),
            path: crate::util::text(&v["path"]),
            size: crate::util::num(&v["size"]),
            files: Cell::new(v["files"].as_f64().filter(|f| f.is_finite())),
            is_dir: v["isDir"] == true,
            other: v["other"] == true,
            count: crate::util::num(&v["count"]),
            truncated: Cell::new(v["truncated"] == true),
            children: RefCell::new(children),
        })
    }

    /// The children with a size, in the host's order (biggest first).
    pub fn kids(&self) -> Vec<Rc<Node>> {
        self.children.borrow().iter().filter(|c| c.size > 0.0).cloned().collect()
    }
}

/// The colour of an arc (sunburst.js arcColor): the top-level hue, lighter with depth; "Overige" neutral.
pub fn arc_color(top: usize, depth: usize, other: bool) -> gdk::RGBA {
    if other {
        return layout::color("tm-cat-other");
    }
    let base = layout::color(&crate::util::ring_color(top));
    if depth <= 1 {
        return base;
    }
    let dark = kit::theme::dark();
    let mixer = kit::theme::rgba(if dark { "#1f1f1f" } else { "#ffffff" });
    let p = match (depth.min(4), dark) {
        (2, true) => 0.78,
        (2, false) => 0.80,
        (3, true) => 0.60,
        (3, false) => 0.62,
        (_, true) => 0.46,
        (_, false) => 0.48,
    };
    layout::mix(&base, p, &mixer)
}

// -------------------------------------------------------------------------------------------- the widget

#[derive(Clone)]
struct Arc_ {
    node: Rc<Node>,
    top: usize,
    depth: usize,
    parent: Option<usize>,
    a0: f64,
    a1: f64,
    ri: f64,
    ro: f64,
}

type NodeFn = Rc<dyn Fn(&Rc<Node>)>;
type HoverFn = Rc<dyn Fn(Option<&Rc<Node>>)>;

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Sunburst {
        pub(super) arcs: RefCell<Vec<Arc_>>,
        pub current: RefCell<Option<Rc<Node>>>,
        pub label: RefCell<String>,
        pub can_up: Cell<bool>,
        pub hot: Cell<Option<usize>>,
        pub hover_center: Cell<bool>,
        pub anim_t0: Cell<i64>,
        pub anim_from: Cell<f64>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        pub on_drill: RefCell<Option<NodeFn>>,
        pub on_pick: RefCell<Option<NodeFn>>,
        pub on_up: RefCell<Option<Rc<dyn Fn()>>>,
        pub on_hover: RefCell<Option<HoverFn>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sunburst {
        const NAME: &'static str = "TaskmgrSunburst";
        type Type = super::Sunburst;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Sunburst {}

    impl WidgetImpl for Sunburst {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return (160, VB as i32, -1, -1);
            }
            let s = if for_size < 0 { VB as i32 } else { for_size.min(VB as i32) };
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.draw(snapshot);
        }
    }

    impl Sunburst {
        /// The square the chart is drawn in: (left, top, side).
        pub fn frame(&self) -> (f64, f64, f64) {
            let obj = self.obj();
            let (w, h) = (obj.width() as f64, obj.height() as f64);
            let side = w.min(h).min(VB).max(1.0);
            ((w - side) / 2.0, (h - side) / 2.0, side)
        }

        fn geometry() -> (f64, f64, f64) {
            let r = VB / 2.0 - 2.0;
            let r0 = r * 0.34;
            (r, r0, (r - r0) / RINGS as f64)
        }

        pub fn build(&self, node: &Rc<Node>) {
            let (_, r0, ring_w) = Self::geometry();
            let mut arcs: Vec<Arc_> = Vec::new();
            fn walk(arcs: &mut Vec<Arc_>, n: &Rc<Node>, a0: f64, a1: f64, depth: usize, top: usize, parent: Option<usize>, r0: f64, ring_w: f64) {
                let kids = n.kids();
                let span = a1 - a0;
                let sum = if n.size > 0.0 { n.size } else { kids.iter().map(|c| c.size).sum::<f64>() }.max(1.0);
                let mut a = a0;
                for (i, c) in kids.iter().enumerate() {
                    let sweep = c.size / sum * span;
                    let (b0, b1) = (a, a + sweep);
                    a = b1;
                    if sweep < if depth == 1 { MIN_SWEEP } else { MIN_SWEEP * 2.0 } {
                        continue;
                    }
                    let top_idx = if depth == 1 { i } else { top };
                    let ri = r0 + (depth - 1) as f64 * ring_w + if depth == 1 { 0.0 } else { 1.0 };
                    let ro = r0 + depth as f64 * ring_w;
                    let pad = (sweep / 6.0).min(0.004);
                    let idx = arcs.len();
                    arcs.push(Arc_ { node: c.clone(), top: top_idx, depth, parent, a0: b0 + pad / 2.0, a1: b1 - pad / 2.0, ri, ro });
                    if depth < RINGS && !c.children.borrow().is_empty() {
                        walk(arcs, c, b0, b1, depth + 1, top_idx, Some(idx), r0, ring_w);
                    }
                }
            }
            walk(&mut arcs, node, -PI / 2.0, -PI / 2.0 + TAU, 1, 0, None, r0, ring_w);
            *self.arcs.borrow_mut() = arcs;
        }

        /// What is under (x, y): Some(Some(arc)) an arc, Some(None) the centre, None nothing.
        pub fn hit(&self, x: f64, y: f64) -> Option<Option<usize>> {
            let (ox, oy, side) = self.frame();
            let s = side / VB;
            let (vx, vy) = ((x - ox) / s - VB / 2.0, (y - oy) / s - VB / 2.0);
            let r = (vx * vx + vy * vy).sqrt();
            let (_, r0, _) = Self::geometry();
            if r < r0 - 3.0 {
                return Some(None);
            }
            let mut ang = vy.atan2(vx);
            if ang < -PI / 2.0 {
                ang += TAU;
            }
            let arcs = self.arcs.borrow();
            arcs.iter().position(|a| r >= a.ri && r <= a.ro && ang >= a.a0 && ang <= a.a1).map(Some)
        }

        fn layout(&self, text: &str, px: f64, weight: gtk::pango::Weight, display: bool, width: f64) -> gtk::pango::Layout {
            let obj = self.obj();
            let layout = obj.create_pango_layout(Some(text));
            let mut fd = obj.pango_context().font_description().unwrap_or_default();
            if display {
                fd.set_family(DISPLAY_FONT);
            }
            fd.set_absolute_size(px * gtk::pango::SCALE as f64);
            fd.set_weight(weight);
            layout.set_font_description(Some(&fd));
            layout.set_width((width.max(10.0) * gtk::pango::SCALE as f64) as i32);
            layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
            layout.set_alignment(gtk::pango::Alignment::Center);
            layout
        }

        fn draw(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let (ox, oy, side) = self.frame();
            let s = side / VB;
            // the drill animation: the rings scale in from 0.82 (in) or 1.12 (out) and fade in over 333 ms
            let now = obj.frame_clock().map(|c| c.frame_time()).unwrap_or(0);
            let t0 = self.anim_t0.get();
            let (scale, ring_alpha, center_alpha) = if t0 > 0 {
                let p = ((now - t0) as f64 / 333_000.0).clamp(0.0, 1.0);
                let e = 1.0 - (1.0 - p).powi(4);
                let from = self.anim_from.get();
                let pc = ((now - t0) as f64 / 250_000.0).clamp(0.0, 1.0);
                (from + (1.0 - from) * e, 0.2 + 0.8 * e, pc)
            } else {
                (1.0, 1.0, 1.0)
            };
            let (_, r0, _) = Self::geometry();
            {
                let cr = snapshot.append_cairo(&gtk::graphene::Rect::new(0.0, 0.0, w, h));
                cr.translate(ox, oy);
                cr.scale(s, s);
                let c = VB / 2.0;
                // the centre's hover
                if self.can_up.get() && self.hover_center.get() {
                    layout::set_source(&cr, &kit::theme::token("fill-subtle-2"));
                    cr.arc(c, c, r0 - 3.0, 0.0, TAU);
                    cr.fill().ok();
                }
                cr.save().ok();
                cr.translate(c, c);
                cr.scale(scale, scale);
                cr.translate(-c, -c);
                let gap = kit::theme::rgba(if kit::theme::dark() { "#2e2e2e" } else { "#fbfbfb" });
                let arcs = self.arcs.borrow();
                let hot = self.hot.get();
                let mut trail = vec![false; arcs.len()];
                if let Some(i) = hot {
                    let mut p = arcs.get(i).and_then(|a| a.parent);
                    while let Some(j) = p {
                        trail[j] = true;
                        p = arcs[j].parent;
                    }
                }
                for (i, a) in arcs.iter().enumerate() {
                    let mut col = arc_color(a.top, a.depth, a.node.other);
                    let alpha = match hot {
                        None => 1.0,
                        Some(h) if h == i => {
                            // brightness(1.08)
                            col = gdk::RGBA::new((col.red() * 1.08).min(1.0), (col.green() * 1.08).min(1.0), (col.blue() * 1.08).min(1.0), col.alpha());
                            1.0
                        }
                        Some(_) if trail[i] => 0.85,
                        Some(_) => 0.38,
                    };
                    cr.new_path();
                    if a.a1 - a.a0 >= TAU - 1e-4 {
                        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
                        cr.arc(c, c, a.ro, 0.0, TAU);
                        cr.new_sub_path();
                        cr.arc(c, c, a.ri, 0.0, TAU);
                    } else {
                        cr.set_fill_rule(gtk::cairo::FillRule::Winding);
                        cr.arc(c, c, a.ro, a.a0, a.a1);
                        cr.arc_negative(c, c, a.ri, a.a1, a.a0);
                        cr.close_path();
                    }
                    cr.set_source_rgba(col.red() as f64, col.green() as f64, col.blue() as f64, col.alpha() as f64 * alpha * ring_alpha);
                    cr.fill_preserve().ok();
                    cr.set_source_rgba(gap.red() as f64, gap.green() as f64, gap.blue() as f64, gap.alpha() as f64 * ring_alpha);
                    cr.set_line_width(1.25);
                    cr.stroke().ok();
                }
                cr.restore().ok();
            }
            // the centre: name, size, number of files
            let Some(node) = self.current.borrow().clone() else { return };
            let hole = (r0 - 3.0) * 2.0 * s;
            let text_w = (hole - 24.0).max(20.0);
            let label = self.label.borrow().clone();
            let name = if label.is_empty() { node.name.clone() } else { label };
            let l_name = self.layout(&name, layout::fs("fs-body", 14.0), gtk::pango::Weight::Semibold, false, text_w);
            let l_size = self.layout(&kit::format::bytes(node.size), layout::fs("fs-subtitle", 20.0), gtk::pango::Weight::Semibold, true, text_w);
            let meta = node.files.get().map(files_text).unwrap_or_default();
            let l_meta = self.layout(&meta, layout::fs("fs-caption", 12.0), gtk::pango::Weight::Normal, false, text_w);
            let up_h = if self.can_up.get() { 12.0 } else { 0.0 };
            let hs: Vec<f64> = [&l_name, &l_size, &l_meta].iter().map(|l| l.pixel_size().1 as f64).collect();
            let meta_h = if meta.is_empty() { 0.0 } else { hs[2] };
            let total = up_h + hs[0] + hs[1] + meta_h;
            let cx = ox + side / 2.0;
            let mut y = oy + side / 2.0 - total / 2.0;
            let t1 = kit::theme::token("text-1");
            let t2 = kit::theme::token("text-2");
            snapshot.push_opacity(center_alpha);
            if self.can_up.get() {
                let cr = snapshot.append_cairo(&gtk::graphene::Rect::new((cx - 8.0) as f32, y as f32, 16.0, 12.0));
                layout::set_source(&cr, &t2);
                cr.set_line_width(1.5);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                cr.move_to(cx - 4.0, y + 6.0);
                cr.line_to(cx, y + 2.0);
                cr.line_to(cx + 4.0, y + 6.0);
                cr.stroke().ok();
                y += up_h;
            }
            for (l, color, hgt) in [(&l_name, &t1, hs[0]), (&l_size, &t1, hs[1]), (&l_meta, &t2, meta_h)] {
                if hgt <= 0.0 {
                    continue;
                }
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new((cx - text_w / 2.0) as f32, y as f32));
                snapshot.append_layout(l, color);
                snapshot.restore();
                y += hgt;
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// The sunburst (see the module docs).
    pub struct Sunburst(ObjectSubclass<imp::Sunburst>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Sunburst {
    fn default() -> Self {
        Self::new()
    }
}

impl Sunburst {
    pub fn new() -> Sunburst {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let sb: Sunburst = glib::Object::new();
        sb.add_css_class("taskmgr-sb");
        sb.set_halign(gtk::Align::Center);
        sb.set_valign(gtk::Align::Start);
        kit::theme::track(&sb);

        let motion = gtk::EventControllerMotion::new();
        let wk = sb.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(sb) = wk.upgrade() {
                let hit = sb.imp().hit(x, y);
                sb.imp().hover_center.set(matches!(hit, Some(None)));
                sb.set_hot(hit.flatten());
                let pointer = match hit {
                    Some(Some(i)) => sb.imp().arcs.borrow().get(i).is_some_and(|a| !a.node.other),
                    Some(None) => sb.imp().can_up.get(),
                    None => false,
                };
                sb.set_cursor_from_name(if pointer { Some("pointer") } else { None });
                sb.queue_draw();
            }
        });
        let wk = sb.downgrade();
        motion.connect_leave(move |_| {
            if let Some(sb) = wk.upgrade() {
                sb.imp().hover_center.set(false);
                sb.set_hot(None);
                sb.queue_draw();
            }
        });
        sb.add_controller(motion);

        let click = gtk::GestureClick::new();
        let wk = sb.downgrade();
        click.connect_released(move |g, _, x, y| {
            let Some(sb) = wk.upgrade() else { return };
            match sb.imp().hit(x, y) {
                Some(Some(i)) => {
                    let Some(node) = sb.imp().arcs.borrow().get(i).map(|a| a.node.clone()) else { return };
                    if node.other {
                        return;
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                    let f = if node.is_dir { sb.imp().on_drill.borrow().clone() } else { sb.imp().on_pick.borrow().clone() };
                    if let Some(f) = f {
                        f(&node);
                    }
                }
                Some(None) => sb.go_up(),
                None => {}
            }
        });
        sb.add_controller(click);

        let keys = gtk::EventControllerKey::new();
        let wk = sb.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(sb) = wk.upgrade() else { return glib::Propagation::Proceed };
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) && sb.imp().can_up.get() {
                sb.go_up();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        sb.add_controller(keys);

        sb.set_has_tooltip(true);
        sb.connect_query_tooltip(|sb, x, y, _, tip| {
            match sb.imp().hit(x as f64, y as f64) {
                Some(Some(i)) => {
                    let Some(a) = sb.imp().arcs.borrow().get(i).cloned() else { return false };
                    let cur = sb.imp().current.borrow().clone();
                    let parent_size = cur.as_ref().map(|c| c.size).unwrap_or(0.0);
                    let pct = if parent_size > 0.0 { a.node.size / parent_size * 100.0 } else { 0.0 };
                    let label = sb.imp().label.borrow().clone();
                    let pname = if label.is_empty() { cur.map(|c| c.name.clone()).unwrap_or_default() } else { label };
                    let b = layout::vbox(0, "taskmgr-sb-tip");
                    let name = layout::wrap(if a.node.other { tr("other") } else { &a.node.name }, "taskmgr-sb-tip-name");
                    name.set_max_width_chars(36);
                    b.append(&name);
                    b.append(&layout::label(&kit::format::bytes(a.node.size), "taskmgr-sb-tip-size"));
                    let p = format!("{}%", kit::format::number(pct, if pct < 10.0 { 1 } else { 0 }));
                    b.append(&layout::label(&crate::util::fill(tr("ofParent"), &[("p", &p), ("name", &pname)]), "taskmgr-sb-tip-meta"));
                    if a.node.is_dir
                        && let Some(f) = a.node.files.get().filter(|f| *f > 0.0)
                    {
                        b.append(&layout::label(&files_text(f), "taskmgr-sb-tip-meta"));
                    }
                    tip.set_custom(Some(&b));
                    true
                }
                Some(None) if sb.imp().can_up.get() => {
                    tip.set_text(Some(tr("up")));
                    true
                }
                _ => false,
            }
        });
        sb
    }

    fn go_up(&self) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        if !self.imp().can_up.get() {
            return;
        }
        let f = self.imp().on_up.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    fn set_hot(&self, idx: Option<usize>) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        if self.imp().hot.get() == idx {
            return;
        }
        self.imp().hot.set(idx);
        self.queue_draw();
        let node = idx.and_then(|i| self.imp().arcs.borrow().get(i).map(|a| a.node.clone()));
        let f = self.imp().on_hover.borrow().clone();
        if let Some(f) = f {
            f(node.as_ref());
        }
    }

    pub fn connect_drill(&self, f: impl Fn(&Rc<Node>) + 'static) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().on_drill.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_pick(&self, f: impl Fn(&Rc<Node>) + 'static) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().on_pick.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_up(&self, f: impl Fn() + 'static) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().on_up.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_hover(&self, f: impl Fn(Option<&Rc<Node>>) + 'static) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().on_hover.borrow_mut() = Some(Rc::new(f));
    }

    /// Show `node`: `direction` "in" / "out" animates, `up` makes the centre go one level up, `label` names the
    /// root ("Persoonlijke map").
    pub fn set_root(&self, node: &Rc<Node>, direction: Option<&str>, up: bool, label: &str) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let imp = self.imp();
        *imp.current.borrow_mut() = Some(node.clone());
        *imp.label.borrow_mut() = label.to_owned();
        imp.can_up.set(up);
        imp.hot.set(None);
        imp.build(node);
        self.set_focusable(up);
        if let Some(t) = imp.tick.borrow_mut().take() {
            t.remove();
        }
        imp.anim_t0.set(0);
        if let Some(dir) = direction
            && !kit::ctx().reduce_motion()
            && let Some(clock) = self.frame_clock()
        {
            imp.anim_from.set(if dir == "in" { 0.82 } else { 1.12 });
            imp.anim_t0.set(clock.frame_time().max(1));
            let id = self.add_tick_callback(|sb, clock| {
                let imp = sb.imp();
                sb.queue_draw();
                if clock.frame_time() - imp.anim_t0.get() >= 340_000 {
                    imp.anim_t0.set(0);
                    imp.tick.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
            *imp.tick.borrow_mut() = Some(id);
        }
        self.queue_draw();
    }

    /// Highlight the arc of a node (from the side list), or none.
    pub fn highlight(&self, node: Option<&Rc<Node>>) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let idx = node.and_then(|n| self.imp().arcs.borrow().iter().position(|a| Rc::ptr_eq(&a.node, n)));
        self.set_hot(idx);
    }

    /// The colour a node is drawn in (the side list's swatches); the neutral colour when it has no arc.
    pub fn color_of(&self, node: &Rc<Node>) -> gdk::RGBA {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        match self.imp().arcs.borrow().iter().find(|a| Rc::ptr_eq(&a.node, node)) {
            Some(a) => arc_color(a.top, a.depth, a.node.other),
            None => layout::color("tm-cat-other"),
        }
    }

    /// The (top, depth, other) of a node's arc, so a swatch can follow the theme.
    pub fn arc_of(&self, node: &Rc<Node>) -> Option<(usize, usize, bool)> {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().arcs.borrow().iter().find(|a| Rc::ptr_eq(&a.node, node)).map(|a| (a.top, a.depth, a.node.other))
    }
}
