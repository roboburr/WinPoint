//! A small SVG renderer for the glyph sources of glyph_data.rs, drawing with cairo in-process (no image loader
//! process, no textures): the elements path, circle, ellipse, rect, g, defs, linearGradient and stop; the
//! attributes fill, stroke, stroke-width, stroke-linecap, stroke-linejoin, opacity, fill-opacity,
//! stroke-opacity and transform (rotate, translate, scale, matrix); full path data including arcs.
use gtk::cairo;
use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq)]
enum Paint {
    None,
    Current,
    Rgba(f64, f64, f64, f64),
    Url(String),
}

#[derive(Clone, Debug)]
struct Style {
    fill: Paint,
    stroke: Paint,
    stroke_width: f64,
    cap: cairo::LineCap,
    join: cairo::LineJoin,
    fill_opacity: f64,
    stroke_opacity: f64,
}

#[derive(Debug, Default)]
pub(crate) struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn num(&self, name: &str) -> f64 {
        self.attr(name).and_then(parse_len).unwrap_or(0.0)
    }
}

fn parse_len(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f64>().ok().map(|v| v / 100.0);
    }
    s.trim_end_matches("px").parse().ok()
}

/// Parse a body (a sequence of elements) into a root group.
pub(crate) fn parse(body: &str) -> Node {
    let mut p = XmlParser { s: body.as_bytes(), i: 0 };
    let mut root = Node { tag: "g".into(), ..Default::default() };
    p.children(&mut root);
    root
}

struct XmlParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl XmlParser<'_> {
    fn children(&mut self, parent: &mut Node) {
        loop {
            while self.i < self.s.len() && self.s[self.i] != b'<' {
                self.i += 1;
            }
            if self.i >= self.s.len() {
                return;
            }
            if self.s.get(self.i + 1) == Some(&b'/') {
                while self.i < self.s.len() && self.s[self.i] != b'>' {
                    self.i += 1;
                }
                self.i += 1;
                return;
            }
            self.i += 1;
            let start = self.i;
            while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'-') {
                self.i += 1;
            }
            let tag = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            let mut node = Node { tag, ..Default::default() };
            let mut closed = false;
            loop {
                while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                    self.i += 1;
                }
                if self.i >= self.s.len() {
                    break;
                }
                match self.s[self.i] {
                    b'/' => {
                        closed = true;
                        while self.i < self.s.len() && self.s[self.i] != b'>' {
                            self.i += 1;
                        }
                        self.i += 1;
                        break;
                    }
                    b'>' => {
                        self.i += 1;
                        break;
                    }
                    _ => {
                        let ks = self.i;
                        while self.i < self.s.len() && !matches!(self.s[self.i], b'=' | b'>' | b'/') && !self.s[self.i].is_ascii_whitespace() {
                            self.i += 1;
                        }
                        let key = String::from_utf8_lossy(&self.s[ks..self.i]).into_owned();
                        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                            self.i += 1;
                        }
                        let mut value = String::new();
                        if self.s.get(self.i) == Some(&b'=') {
                            self.i += 1;
                            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                                self.i += 1;
                            }
                            if let Some(&q) = self.s.get(self.i)
                                && (q == b'"' || q == b'\'')
                            {
                                self.i += 1;
                                let vs = self.i;
                                while self.i < self.s.len() && self.s[self.i] != q {
                                    self.i += 1;
                                }
                                value = String::from_utf8_lossy(&self.s[vs..self.i]).into_owned();
                                self.i += 1;
                            }
                        }
                        if key.is_empty() {
                            self.i += 1;
                        } else {
                            node.attrs.push((key, value));
                        }
                    }
                }
            }
            if !closed {
                self.children(&mut node);
            }
            parent.children.push(node);
        }
    }
}

fn parse_paint(v: &str) -> Option<Paint> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if v == "none" || v == "transparent" {
        return Some(Paint::None);
    }
    if v == "currentColor" {
        return Some(Paint::Current);
    }
    if let Some(id) = v.strip_prefix("url(#").and_then(|r| r.strip_suffix(')')) {
        return Some(Paint::Url(id.to_owned()));
    }
    parse_color(v).map(|(r, g, b, a)| Paint::Rgba(r, g, b, a))
}

fn parse_color(v: &str) -> Option<(f64, f64, f64, f64)> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        let full: String = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect(),
            6 => h.to_owned(),
            _ => return None,
        };
        let n = u32::from_str_radix(&full, 16).ok()?;
        return Some((((n >> 16) & 255) as f64 / 255.0, ((n >> 8) & 255) as f64 / 255.0, (n & 255) as f64 / 255.0, 1.0));
    }
    match v {
        "white" => Some((1.0, 1.0, 1.0, 1.0)),
        "black" => Some((0.0, 0.0, 0.0, 1.0)),
        _ => gtk::gdk::RGBA::parse(v)
            .ok()
            .map(|c| (c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64)),
    }
}

/// Draw a parsed glyph into `cr`, whose user space is the glyph's view box. `outline` applies the icons.js
/// defaults (fill none, stroke currentColor with `stroke` width, round caps and joins).
pub(crate) fn render(cr: &cairo::Context, root: &Node, color: (f64, f64, f64, f64), outline: bool, stroke: f64) {
    let style = if outline {
        Style {
            fill: Paint::None,
            stroke: Paint::Current,
            stroke_width: stroke,
            cap: cairo::LineCap::Round,
            join: cairo::LineJoin::Round,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
        }
    } else {
        Style {
            fill: Paint::Rgba(0.0, 0.0, 0.0, 1.0),
            stroke: Paint::None,
            stroke_width: 1.0,
            cap: cairo::LineCap::Butt,
            join: cairo::LineJoin::Miter,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
        }
    };
    let mut gradients = HashMap::new();
    collect_gradients(root, &mut gradients);
    let r = Renderer { color, gradients };
    r.node(cr, root, &style, 1.0);
}

struct Gradient {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    stops: Vec<(f64, (f64, f64, f64, f64))>,
}

fn collect_gradients(n: &Node, out: &mut HashMap<String, Gradient>) {
    if n.tag == "linearGradient"
        && let Some(id) = n.attr("id")
    {
        let stops = n
            .children
            .iter()
            .filter(|c| c.tag == "stop")
            .map(|s| {
                let (r, g, b, _) = s.attr("stop-color").and_then(parse_color).unwrap_or((0.0, 0.0, 0.0, 1.0));
                let a = s.attr("stop-opacity").and_then(parse_len).unwrap_or(1.0);
                (s.attr("offset").and_then(parse_len).unwrap_or(0.0), (r, g, b, a))
            })
            .collect();
        out.insert(
            id.to_owned(),
            Gradient { x1: n.num("x1"), y1: n.num("y1"), x2: n.num("x2"), y2: n.num("y2"), stops },
        );
    }
    for c in &n.children {
        collect_gradients(c, out);
    }
}

struct Renderer {
    color: (f64, f64, f64, f64),
    gradients: HashMap<String, Gradient>,
}

impl Renderer {
    fn node(&self, cr: &cairo::Context, n: &Node, parent: &Style, parent_opacity: f64) {
        if matches!(n.tag.as_str(), "defs" | "linearGradient" | "radialGradient" | "stop" | "title") {
            return;
        }
        let mut st = parent.clone();
        if let Some(p) = n.attr("fill").and_then(parse_paint) {
            st.fill = p;
        }
        if let Some(p) = n.attr("stroke").and_then(parse_paint) {
            st.stroke = p;
        }
        if let Some(w) = n.attr("stroke-width").and_then(parse_len) {
            st.stroke_width = w;
        }
        match n.attr("stroke-linecap") {
            Some("round") => st.cap = cairo::LineCap::Round,
            Some("square") => st.cap = cairo::LineCap::Square,
            Some("butt") => st.cap = cairo::LineCap::Butt,
            _ => {}
        }
        match n.attr("stroke-linejoin") {
            Some("round") => st.join = cairo::LineJoin::Round,
            Some("bevel") => st.join = cairo::LineJoin::Bevel,
            Some("miter") => st.join = cairo::LineJoin::Miter,
            _ => {}
        }
        if let Some(v) = n.attr("fill-opacity").and_then(parse_len) {
            st.fill_opacity = v;
        }
        if let Some(v) = n.attr("stroke-opacity").and_then(parse_len) {
            st.stroke_opacity = v;
        }
        let opacity = parent_opacity * n.attr("opacity").and_then(parse_len).unwrap_or(1.0);
        let saved = cr.matrix();
        if let Some(t) = n.attr("transform") {
            apply_transform(cr, t);
        }
        match n.tag.as_str() {
            "g" | "svg" => {
                for c in &n.children {
                    self.node(cr, c, &st, opacity);
                }
            }
            "path" => {
                cr.new_path();
                path(cr, n.attr("d").unwrap_or(""));
                self.paint(cr, &st, opacity, saved);
            }
            "circle" => {
                cr.new_path();
                cr.new_sub_path();
                cr.arc(n.num("cx"), n.num("cy"), n.num("r").max(0.0), 0.0, 2.0 * PI);
                cr.close_path();
                self.paint(cr, &st, opacity, saved);
            }
            "ellipse" => {
                cr.new_path();
                let (rx, ry) = (n.num("rx"), n.num("ry"));
                if rx > 0.0 && ry > 0.0 {
                    let m = cr.matrix();
                    cr.translate(n.num("cx"), n.num("cy"));
                    cr.scale(rx, ry);
                    cr.new_sub_path();
                    cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI);
                    cr.close_path();
                    cr.set_matrix(m);
                }
                self.paint(cr, &st, opacity, saved);
            }
            "rect" => {
                cr.new_path();
                let (x, y, w, h) = (n.num("x"), n.num("y"), n.num("width"), n.num("height"));
                let mut rx = n.attr("rx").and_then(parse_len);
                let mut ry = n.attr("ry").and_then(parse_len);
                if rx.is_none() {
                    rx = ry;
                }
                if ry.is_none() {
                    ry = rx;
                }
                let rx = rx.unwrap_or(0.0).clamp(0.0, w / 2.0);
                let ry = ry.unwrap_or(0.0).clamp(0.0, h / 2.0);
                if w > 0.0 && h > 0.0 {
                    rounded_rect(cr, x, y, w, h, rx, ry);
                }
                self.paint(cr, &st, opacity, saved);
            }
            _ => {}
        }
        cr.set_matrix(saved);
    }

    /// Fill and stroke the current path. Strokes are drawn in the element's own user space (the transform
    /// in effect), like SVG.
    fn paint(&self, cr: &cairo::Context, st: &Style, opacity: f64, _outer: cairo::Matrix) {
        if st.fill != Paint::None {
            self.source(cr, &st.fill, st.fill_opacity * opacity);
            let _ = cr.fill_preserve();
        }
        if st.stroke != Paint::None && st.stroke_width > 0.0 {
            self.source(cr, &st.stroke, st.stroke_opacity * opacity);
            cr.set_line_width(st.stroke_width);
            cr.set_line_cap(st.cap);
            cr.set_line_join(st.join);
            let _ = cr.stroke_preserve();
        }
        cr.new_path();
    }

    fn source(&self, cr: &cairo::Context, paint: &Paint, alpha: f64) {
        match paint {
            Paint::Current | Paint::None => {
                let (r, g, b, a) = self.color;
                cr.set_source_rgba(r, g, b, a * alpha);
            }
            Paint::Rgba(r, g, b, a) => cr.set_source_rgba(*r, *g, *b, a * alpha),
            Paint::Url(id) => match self.gradients.get(id) {
                Some(gr) => {
                    let lg = cairo::LinearGradient::new(gr.x1, gr.y1, gr.x2, gr.y2);
                    for (off, (r, g, b, a)) in &gr.stops {
                        lg.add_color_stop_rgba(*off, *r, *g, *b, a * alpha);
                    }
                    let _ = cr.set_source(&lg);
                }
                None => {
                    let (r, g, b, a) = self.color;
                    cr.set_source_rgba(r, g, b, a * alpha);
                }
            },
        }
    }
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) {
    cr.new_sub_path();
    if rx <= 0.0 || ry <= 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    let m = cr.matrix();
    let corner = |cx: f64, cy: f64, a1: f64, a2: f64| {
        cr.save().ok();
        cr.translate(cx, cy);
        cr.scale(rx, ry);
        cr.arc(0.0, 0.0, 1.0, a1, a2);
        cr.restore().ok();
    };
    corner(x + w - rx, y + ry, -PI / 2.0, 0.0);
    corner(x + w - rx, y + h - ry, 0.0, PI / 2.0);
    corner(x + rx, y + h - ry, PI / 2.0, PI);
    corner(x + rx, y + ry, PI, 1.5 * PI);
    cr.close_path();
    cr.set_matrix(m);
}

fn apply_transform(cr: &cairo::Context, t: &str) {
    let mut rest = t.trim();
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { return };
        let args: Vec<f64> = rest[open + 1..open + close]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        match (name, args.as_slice()) {
            ("rotate", [a]) => cr.rotate(a.to_radians()),
            ("rotate", [a, cx, cy]) => {
                cr.translate(*cx, *cy);
                cr.rotate(a.to_radians());
                cr.translate(-cx, -cy);
            }
            ("translate", [x]) => cr.translate(*x, 0.0),
            ("translate", [x, y]) => cr.translate(*x, *y),
            ("scale", [s]) => cr.scale(*s, *s),
            ("scale", [x, y]) => cr.scale(*x, *y),
            ("matrix", [a, b, c, d, e, f]) => cr.transform(cairo::Matrix::new(*a, *b, *c, *d, *e, *f)),
            _ => {}
        }
        rest = &rest[open + close + 1..];
    }
}

// ------------------------------------------------------------------------------------------- path data

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn at_end(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }
    fn peek_command(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied().filter(|c| c.is_ascii_alphabetic() && *c != b'e' && *c != b'E')
    }
    fn number(&mut self) -> Option<f64> {
        self.skip();
        let start = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'+' || s[self.i] == b'-') {
            self.i += 1;
        }
        let mut digits = false;
        let mut dot = false;
        while self.i < s.len() {
            let c = s[self.i];
            if c.is_ascii_digit() {
                digits = true;
                self.i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if digits && self.i < s.len() && (s[self.i] == b'e' || s[self.i] == b'E') {
            let mut j = self.i + 1;
            if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
                j += 1;
            }
            if j < s.len() && s[j].is_ascii_digit() {
                while j < s.len() && s[j].is_ascii_digit() {
                    j += 1;
                }
                self.i = j;
            }
        }
        if !digits {
            self.i = start;
            return None;
        }
        std::str::from_utf8(&s[start..self.i]).ok()?.parse().ok()
    }
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Some(false)
            }
            Some(b'1') => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }
}

fn path(cr: &cairo::Context, d: &str) {
    let mut lx = Lexer { s: d.as_bytes(), i: 0 };
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    // The last cubic / quadratic control point, for S and T.
    let mut last_c: Option<(f64, f64)> = None;
    let mut last_q: Option<(f64, f64)> = None;
    let mut cmd: u8 = 0;
    loop {
        if lx.at_end() {
            break;
        }
        if let Some(c) = lx.peek_command() {
            cmd = c;
            lx.i += 1;
            if cmd == b'Z' || cmd == b'z' {
                cr.close_path();
                x = sx;
                y = sy;
                last_c = None;
                last_q = None;
                continue;
            }
        } else if cmd == 0 || cmd == b'Z' || cmd == b'z' {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let before = lx.i;
        let ok = match cmd.to_ascii_uppercase() {
            b'M' => match (lx.number(), lx.number()) {
                (Some(a), Some(b)) => {
                    x = ox + a;
                    y = oy + b;
                    cr.move_to(x, y);
                    sx = x;
                    sy = y;
                    // further pairs are implicit line-tos
                    cmd = if rel { b'l' } else { b'L' };
                    last_c = None;
                    last_q = None;
                    true
                }
                _ => false,
            },
            b'L' => match (lx.number(), lx.number()) {
                (Some(a), Some(b)) => {
                    x = ox + a;
                    y = oy + b;
                    cr.line_to(x, y);
                    last_c = None;
                    last_q = None;
                    true
                }
                _ => false,
            },
            b'H' => match lx.number() {
                Some(a) => {
                    x = if rel { x + a } else { a };
                    cr.line_to(x, y);
                    last_c = None;
                    last_q = None;
                    true
                }
                None => false,
            },
            b'V' => match lx.number() {
                Some(a) => {
                    y = if rel { y + a } else { a };
                    cr.line_to(x, y);
                    last_c = None;
                    last_q = None;
                    true
                }
                None => false,
            },
            b'C' => match (lx.number(), lx.number(), lx.number(), lx.number(), lx.number(), lx.number()) {
                (Some(x1), Some(y1), Some(x2), Some(y2), Some(ex), Some(ey)) => {
                    let (x1, y1, x2, y2, ex, ey) = (ox + x1, oy + y1, ox + x2, oy + y2, ox + ex, oy + ey);
                    cr.curve_to(x1, y1, x2, y2, ex, ey);
                    last_c = Some((x2, y2));
                    last_q = None;
                    x = ex;
                    y = ey;
                    true
                }
                _ => false,
            },
            b'S' => match (lx.number(), lx.number(), lx.number(), lx.number()) {
                (Some(x2), Some(y2), Some(ex), Some(ey)) => {
                    let (x1, y1) = match last_c {
                        Some((cx, cy)) => (2.0 * x - cx, 2.0 * y - cy),
                        None => (x, y),
                    };
                    let (x2, y2, ex, ey) = (ox + x2, oy + y2, ox + ex, oy + ey);
                    cr.curve_to(x1, y1, x2, y2, ex, ey);
                    last_c = Some((x2, y2));
                    last_q = None;
                    x = ex;
                    y = ey;
                    true
                }
                _ => false,
            },
            b'Q' => match (lx.number(), lx.number(), lx.number(), lx.number()) {
                (Some(qx), Some(qy), Some(ex), Some(ey)) => {
                    let (qx, qy, ex, ey) = (ox + qx, oy + qy, ox + ex, oy + ey);
                    quad(cr, x, y, qx, qy, ex, ey);
                    last_q = Some((qx, qy));
                    last_c = None;
                    x = ex;
                    y = ey;
                    true
                }
                _ => false,
            },
            b'T' => match (lx.number(), lx.number()) {
                (Some(ex), Some(ey)) => {
                    let (qx, qy) = match last_q {
                        Some((cx, cy)) => (2.0 * x - cx, 2.0 * y - cy),
                        None => (x, y),
                    };
                    let (ex, ey) = (ox + ex, oy + ey);
                    quad(cr, x, y, qx, qy, ex, ey);
                    last_q = Some((qx, qy));
                    last_c = None;
                    x = ex;
                    y = ey;
                    true
                }
                _ => false,
            },
            b'A' => {
                let rx = lx.number();
                let ry = lx.number();
                let rot = lx.number();
                let large = lx.flag();
                let sweep = lx.flag();
                let ex = lx.number();
                let ey = lx.number();
                match (rx, ry, rot, large, sweep, ex, ey) {
                    (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep), Some(ex), Some(ey)) => {
                        let (ex, ey) = (ox + ex, oy + ey);
                        arc(cr, x, y, rx, ry, rot, large, sweep, ex, ey);
                        x = ex;
                        y = ey;
                        last_c = None;
                        last_q = None;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !ok || lx.i == before {
            break;
        }
    }
}

fn quad(cr: &cairo::Context, x0: f64, y0: f64, qx: f64, qy: f64, x: f64, y: f64) {
    cr.curve_to(
        x0 + 2.0 / 3.0 * (qx - x0),
        y0 + 2.0 / 3.0 * (qy - y0),
        x + 2.0 / 3.0 * (qx - x),
        y + 2.0 / 3.0 * (qy - y),
        x,
        y,
    );
}

/// An SVG elliptical arc from the current point (SVG 1.1 F.6.5, endpoint to center parameterization).
#[allow(clippy::too_many_arguments)]
fn arc(cr: &cairo::Context, x1: f64, y1: f64, rx: f64, ry: f64, rot: f64, large: bool, sweep: bool, x2: f64, y2: f64) {
    if (x1 - x2).abs() < 1e-9 && (y1 - y2).abs() < 1e-9 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-9 || ry < 1e-9 {
        cr.line_to(x2, y2);
        return;
    }
    let phi = rot.to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = cos * dx + sin * dy;
    let y1p = -sin * dx + cos * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos * cxp - sin * cyp + (x1 + x2) / 2.0;
    let cy = sin * cxp + cos * cyp + (y1 + y2) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * PI;
    }
    let m = cr.matrix();
    cr.translate(cx, cy);
    cr.rotate(phi);
    cr.scale(rx, ry);
    if dtheta > 0.0 {
        cr.arc(0.0, 0.0, 1.0, theta1, theta1 + dtheta);
    } else {
        cr.arc_negative(0.0, 0.0, 1.0, theta1, theta1 + dtheta);
    }
    cr.set_matrix(m);
}
