//! The matched parts of a name in semibold (highlight.js; Everything shows them in bold): the search's
//! highlight {terms, regex, case} from index.search, as Pango attributes on the cell's label.
use gtk::glib;
use gtk::pango;
use serde_json::Value;
use std::cell::RefCell;

/// index.search's highlight, plus whether the path column gets it too (matchPath or a path: modifier).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Highlight {
    pub terms: Vec<String>,
    pub regex: Option<String>,
    pub case: bool,
    pub path: bool,
}

impl Highlight {
    pub fn from_value(v: &Value, path: bool) -> Option<Highlight> {
        if !v.is_object() {
            return None;
        }
        Some(Highlight {
            terms: v["terms"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default(),
            regex: v["regex"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
            case: v["case"] == true,
            path,
        })
    }
}

thread_local! {
    static RX: RefCell<Option<(String, bool, Option<glib::Regex>)>> = const { RefCell::new(None) };
}

fn regex_for(src: &str, case: bool) -> Option<glib::Regex> {
    RX.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((s, cs, rx)) = c.as_ref()
            && s == src
            && *cs == case
        {
            return rx.clone();
        }
        let mut flags = glib::RegexCompileFlags::OPTIMIZE;
        if !case {
            flags |= glib::RegexCompileFlags::CASELESS;
        }
        let rx = glib::Regex::new(src, flags, glib::RegexMatchFlags::empty()).ok().flatten();
        *c = Some((src.to_owned(), case, rx.clone()));
        rx
    })
}

/// The matched [start, end) byte ranges in `name`, merged and sorted (highlight.js matchRanges).
pub fn match_ranges(name: &str, hl: &Highlight) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    if name.is_empty() {
        return out;
    }
    // Characters (lowercased one by one unless case matters) with their byte offsets. A character that
    // lowercases to several gives up on the terms, as highlight.js does when the length changes.
    let offsets: Vec<usize> = name.char_indices().map(|(i, _)| i).collect();
    let mut same_len = true;
    let hay: Vec<char> = name
        .chars()
        .map(|c| {
            if hl.case {
                return c;
            }
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(a), None) => a,
                _ => {
                    same_len = false;
                    c
                }
            }
        })
        .collect();
    let byte_at = |ci: usize| if ci < offsets.len() { offsets[ci] } else { name.len() };
    if same_len {
        for term in &hl.terms {
            let needle: Vec<char> = if hl.case { term.chars().collect() } else { term.chars().flat_map(char::to_lowercase).collect() };
            if needle.is_empty() || needle.len() > hay.len() + 2 {
                continue;
            }
            let mut p = 0usize;
            let mut guard = 0;
            while p + needle.len() <= hay.len() && guard < 50 {
                if hay[p..p + needle.len()] == needle[..] {
                    out.push((byte_at(p), byte_at(p + needle.len())));
                    guard += 1;
                    p += needle.len();
                } else {
                    p += 1;
                }
            }
        }
    }
    if let Some(src) = &hl.regex
        && let Some(rx) = regex_for(src, hl.case)
    {
        let g = glib::GString::from(name);
        if let Ok(mi) = rx.match_(g.as_gstr(), glib::RegexMatchFlags::empty()) {
            let mut guard = 0;
            while mi.matches() && guard < 50 {
                guard += 1;
                if let Some((a, b)) = mi.fetch_pos(0)
                    && b > a
                    && a >= 0
                {
                    out.push((a as usize, b as usize));
                }
                if mi.next().is_err() {
                    break;
                }
            }
        }
    }
    if out.len() < 2 {
        return out;
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut merged: Vec<(usize, usize)> = vec![out[0]];
    for r in out.into_iter().skip(1) {
        let last = merged.len() - 1;
        if r.0 <= merged[last].1 {
            merged[last].1 = merged[last].1.max(r.1);
        } else {
            merged.push(r);
        }
    }
    merged
}

/// Set `text` on `label` with the matches in semibold (in `color` when given: the path column's matches are
/// in the primary text colour, .alles-hl).
pub fn set_highlighted(label: &gtk::Label, text: &str, hl: Option<&Highlight>, color: Option<gtk::gdk::RGBA>) {
    let ranges = hl.map(|h| match_ranges(text, h)).unwrap_or_default();
    if label.label() != text {
        label.set_label(text);
    }
    if ranges.is_empty() {
        if label.attributes().is_some() {
            label.set_attributes(None);
        }
        return;
    }
    let attrs = pango::AttrList::new();
    for (a, b) in ranges {
        let mut w = pango::AttrInt::new_weight(pango::Weight::Semibold);
        w.set_start_index(a as u32);
        w.set_end_index(b as u32);
        attrs.insert(w);
        if let Some(c) = &color {
            let to16 = |f: f32| (f.clamp(0.0, 1.0) * 65535.0).round() as u16;
            let mut fg = pango::AttrColor::new_foreground(to16(c.red()), to16(c.green()), to16(c.blue()));
            fg.set_start_index(a as u32);
            fg.set_end_index(b as u32);
            attrs.insert(fg);
        }
    }
    label.set_attributes(Some(&attrs));
}
