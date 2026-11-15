//! The disks.list snapshot as the app reads it (6ebf169f:vendor/ui/js/apps/disks/util.js, overview.js layout,
//! main.js findVolume / findDisk, numparse.js): JS truthiness over serde_json values, labels, the bar's layout.
use crate::strings::{s, tf};
use kit::format;
use serde_json::Value;

// ------------------------------------------------------------------------------------------- value helpers

/// JS truthiness: null, false, 0, "" and missing are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// A string field ("" when missing or not a string).
pub fn st(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A number (0 when missing).
pub fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// A number, or None when missing / null.
pub fn opt_num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// The items of an array field (none when missing).
pub fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

// ------------------------------------------------------------------------------------------- lookups

/// main.js findVolume: by id, device, cleartext object or label.
pub fn find_volume(snap: &Value, key: &str) -> Option<(Value, Value)> {
    if key.is_empty() {
        return None;
    }
    for d in arr(&snap["disks"]) {
        for v in arr(&d["volumes"]) {
            if st(&v["id"]) == key
                || st(&v["device"]) == key
                || st(&v["encrypted"]["cleartext"]) == key
                || (v["label"].is_string() && st(&v["label"]) == key)
            {
                return Some((d.clone(), v.clone()));
            }
        }
    }
    None
}

/// main.js findDisk: by id, block, device or name.
pub fn find_disk(snap: &Value, key: &str) -> Option<Value> {
    if key.is_empty() {
        return None;
    }
    arr(&snap["disks"])
        .iter()
        .find(|d| st(&d["id"]) == key || st(&d["block"]) == key || st(&d["device"]) == key || st(&d["name"]) == key)
        .cloned()
}

// ------------------------------------------------------------------------------------------- labels

/// Segment kind for colours: data, system, container, swap, encrypted, free, raw, busy.
pub fn seg_kind(v: &Value) -> &'static str {
    if truthy(&v["busy"]) {
        "busy"
    } else if truthy(&v["encrypted"]) {
        "encrypted"
    } else if truthy(&v["isSwap"]) {
        "swap"
    } else if v["role"] == "container" {
        "container"
    } else if v["role"] == "system" {
        "system"
    } else if !truthy(&v["fsType"]) {
        "raw"
    } else {
        "data"
    }
}

pub fn kind_label(d: &Value) -> String {
    let k = s(&format!("k_{}", st(&d["kind"])));
    let rr = num(&d["rotationRate"]);
    if d["kind"] == "hdd" && rr > 0.0 {
        return format!("{k} ({})", tf("rpm", &[("n", &format::number(rr, 0))]));
    }
    k
}

pub fn table_label(d: &Value) -> String {
    if truthy(&d["partitionTable"]) { s(&st(&d["partitionTable"])) } else { s("noTable") }
}

pub fn table_long(d: &Value) -> String {
    if truthy(&d["partitionTable"]) { s(&format!("{}Long", st(&d["partitionTable"]))) } else { s("noTable") }
}

pub fn bus_label(d: &Value) -> String {
    let bus = st(&d["bus"]);
    let kind = st(&d["kind"]);
    if kind == "nvme" {
        s("bus_nvme")
    } else if bus == "usb" {
        s("bus_usb")
    } else if bus == "sdio" {
        s("bus_sdio")
    } else if bus == "ata" || (bus.is_empty() && (kind == "hdd" || kind == "ssd")) {
        s("bus_sata")
    } else if !bus.is_empty() {
        bus.to_uppercase()
    } else {
        s("bus_other")
    }
}

/// ok, warning, critical or unknown.
pub fn health_of(d: &Value) -> String {
    let smart = &d["smart"];
    if truthy(&smart["supported"]) {
        let status = st(&smart["status"]);
        if status.is_empty() { "unknown".into() } else { status }
    } else {
        "unknown".into()
    }
}

/// Short mount point for titles: "/" "/home" "/srv/vm", not /run/media/<user>/<UUID> (jargon).
pub fn title_with_mount(v: &Value) -> String {
    let name = st(&v["name"]);
    let mp = st(&v["mountPoint"]);
    if mp.is_empty()
        || mp.starts_with("/run/media/")
        || mp.starts_with("/media/")
        || mp.starts_with("/mnt/")
        || mp.encode_utf16().count() > 24
    {
        return name;
    }
    format!("{name} ({mp})")
}

/// The word the user types to confirm a destructive action.
pub fn confirm_word(v: &Value) -> String {
    let label = st(&v["label"]);
    if label.is_empty() { st(&v["name"]).trim().to_owned() } else { label.trim().to_owned() }
}

/// A UDisks job runs on the disk or on one of its volumes: no safe removal, no initialise.
pub fn disk_busy(d: &Value) -> bool {
    truthy(&d["busy"]) || arr(&d["volumes"]).iter().any(|v| truthy(&v["busy"]))
}

pub fn busy_text(busy: &Value) -> String {
    if !truthy(busy) {
        return String::new();
    }
    let op = st(&busy["operation"]);
    let key = if op.contains("erase") {
        "b_erase"
    } else if op.starts_with("format") {
        "b_format"
    } else if op.contains("delete") {
        "b_delete"
    } else if op.contains("create") {
        "b_create"
    } else {
        "b_other"
    };
    let p = busy["progress"].as_f64().map(|p| format!("{}%", (p * 100.0).round() as i64)).unwrap_or_default();
    tf(key, &[("p", &p)]).trim().to_owned()
}

pub fn volume_status(v: &Value) -> String {
    if truthy(&v["busy"]) {
        return busy_text(&v["busy"]);
    }
    if truthy(&v["encrypted"]) && !truthy(&v["encrypted"]["unlocked"]) {
        return s("s_locked");
    }
    if truthy(&v["isSwap"]) {
        return if truthy(&v["swapActive"]) { s("s_swapOn") } else { s("s_swapOff") };
    }
    if !truthy(&v["fsType"]) && v["role"] == "data" {
        return s("s_raw");
    }
    s("s_ok")
}

pub fn bytes(v: &Value) -> String {
    format::bytes(num(v))
}

// ------------------------------------------------------------------------------------------- the bar's layout

/// One place on a disk: a volume or a free region, with an extended partition's contents as children.
#[derive(Clone, Debug)]
pub struct Item {
    pub volume: Option<Value>,
    pub free: Option<Value>,
    pub offset: f64,
    pub size: f64,
    pub children: Vec<Item>,
}

impl Item {
    pub fn is_volume(&self) -> bool {
        self.volume.is_some()
    }
    /// The volume (or Null for a free region).
    pub fn v(&self) -> &Value {
        self.volume.as_ref().unwrap_or(&Value::Null)
    }
    /// The free region (or Null for a volume).
    pub fn f(&self) -> &Value {
        self.free.as_ref().unwrap_or(&Value::Null)
    }
    pub fn is_container(&self) -> bool {
        self.is_volume() && truthy(&self.v()["isContainer"])
    }
}

/// overview.js layout: the disk in offset order; primary / GPT partitions, extended partitions (with their
/// logical drives and the free space inside as children) and unallocated space.
pub fn layout(d: &Value) -> Vec<Item> {
    let containers: Vec<&Value> = arr(&d["volumes"]).iter().filter(|v| truthy(&v["isContainer"])).collect();
    let holder = |offset: f64| -> Option<String> {
        containers.iter().find(|c| num(&c["offset"]) <= offset && offset < num(&c["offset"]) + num(&c["size"])).map(|c| st(&c["id"]))
    };
    let mut inner: Vec<(String, Vec<Item>)> = Vec::new();
    let mut put = |id: String, it: Item| match inner.iter_mut().find(|(k, _)| *k == id) {
        Some((_, list)) => list.push(it),
        None => inner.push((id, vec![it])),
    };
    let mut top = Vec::new();
    for v in arr(&d["volumes"]) {
        let it = Item { volume: Some(v.clone()), free: None, offset: num(&v["offset"]), size: num(&v["size"]), children: vec![] };
        match if truthy(&v["isContained"]) { holder(it.offset) } else { None } {
            Some(c) => put(c, it),
            None => top.push(it),
        }
    }
    for f in arr(&d["free"]) {
        let it = Item { volume: None, free: Some(f.clone()), offset: num(&f["offset"]), size: num(&f["size"]), children: vec![] };
        let c = if truthy(&f["logical"]) {
            containers.iter().find(|x| x["id"] == f["container"] && truthy(&f["container"])).map(|x| st(&x["id"])).or_else(|| holder(it.offset))
        } else {
            None
        };
        match c {
            Some(c) => put(c, it),
            None => top.push(it),
        }
    }
    let by_offset = |a: &Item, b: &Item| a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal);
    top.sort_by(by_offset);
    for it in top.iter_mut() {
        if it.is_container() {
            let id = st(&it.v()["id"]);
            let mut kids = inner.iter().find(|(k, _)| *k == id).map(|(_, l)| l.clone()).unwrap_or_default();
            kids.sort_by(by_offset);
            it.children = kids;
        }
    }
    top
}

/// Flat list in display order (container, then its contents).
pub fn flat_layout(d: &Value) -> Vec<Item> {
    let mut out = Vec::new();
    for it in layout(d) {
        let kids = it.children.clone();
        out.push(it);
        out.extend(kids);
    }
    out
}

/// The selection id of a place: the volume id, or "free:<disk>:<offset>".
pub fn seg_id(d: &Value, it: &Item) -> String {
    if it.is_volume() { st(&it.v()["id"]) } else { free_id(d, it.f()) }
}

pub fn free_id(d: &Value, f: &Value) -> String {
    format!("free:{}:{}", st(&d["id"]), js_num(num(&f["offset"])))
}

/// A number the way JS prints it in a template string (integers without ".0").
fn js_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { format!("{}", n as i64) } else { format!("{n}") }
}

// ------------------------------------------------------------------------------------------- numparse.js

/// Read a number the way the current language writes it: "1.234,56" (nl) / "1,234.56" (en) is 1234.56; the
/// group separator only groups when every group has three digits, otherwise a single one was meant as the
/// decimal point ("500.5" typed in Dutch is 500.5, not 5005). Anything else is NaN.
pub fn parse_locale_number(text: &str) -> f64 {
    let (group, decimal) = if kit::i18n::en() { (',', '.') } else { ('.', ',') };
    let s: String = text.trim().chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}').collect();
    if s.is_empty() {
        return f64::NAN;
    }
    let parts: Vec<&str> = s.split(decimal).collect();
    if parts.len() > 2 {
        return f64::NAN;
    }
    let mut int = parts[0].to_owned();
    let mut frac: Option<String> = parts.get(1).map(|f| (*f).to_owned());
    if int.contains(group) {
        let chunks: Vec<&str> = int.split(group).collect();
        let first_ok = {
            let c = chunks[0].strip_prefix('+').unwrap_or(chunks[0]);
            (1..=3).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_digit())
        };
        if first_ok && chunks[1..].iter().all(|c| c.len() == 3 && c.chars().all(|ch| ch.is_ascii_digit())) {
            int = chunks.concat();
        } else if chunks.len() == 2 && frac.is_none() {
            let (a, b) = (chunks[0].to_owned(), chunks[1].to_owned());
            int = a;
            frac = Some(b);
        } else {
            return f64::NAN;
        }
    }
    let n = match &frac {
        Some(f) => format!("{int}.{f}"),
        None => int,
    };
    // /^\+?(\d+(\.\d*)?|\.\d+)$/
    let body = n.strip_prefix('+').unwrap_or(&n);
    let valid = match body.split_once('.') {
        Some((a, b)) => (!a.is_empty() && a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
            || (a.is_empty() && !b.is_empty() && b.chars().all(|c| c.is_ascii_digit())),
        None => !body.is_empty() && body.chars().all(|c| c.is_ascii_digit()),
    };
    if !valid {
        return f64::NAN;
    }
    body.parse::<f64>().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    // numparse.js's examples, in both languages (kit::i18n follows WIN11_LANG when no settings say otherwise)
    #[test]
    fn locale_numbers() {
        let nl = |t: &str| if kit::i18n::en() { f64::NAN } else { parse_locale_number(t) };
        if !kit::i18n::en() {
            assert_eq!(nl("1.234,56"), 1234.56);
            assert_eq!(nl("500,5"), 500.5);
            assert_eq!(nl("500.5"), 500.5);
            assert_eq!(nl(" 1 024 "), 1024.0);
            assert!(nl("1,2,3").is_nan());
            assert!(nl("abc").is_nan());
            assert!(nl("").is_nan());
        }
    }

    #[test]
    fn layout_nests_logical_drives() {
        let d = serde_json::json!({
            "id": "/d", "size": 1000,
            "volumes": [
                {"id": "/d1", "offset": 0, "size": 300},
                {"id": "/d2", "offset": 300, "size": 600, "isContainer": true},
                {"id": "/d5", "offset": 301, "size": 200, "isContained": true}
            ],
            "free": [{"offset": 600, "size": 300, "logical": true, "container": "/d2"}, {"offset": 900, "size": 100}]
        });
        let top = layout(&d);
        assert_eq!(top.len(), 3);
        assert_eq!(top[1].children.len(), 2);
        let ids: Vec<String> = flat_layout(&d).iter().map(|it| seg_id(&d, it)).collect();
        assert_eq!(ids, ["/d1", "/d2", "/d5", "free:/d:600", "free:/d:900"]);
    }
}
