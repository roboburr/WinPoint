use glib::Variant;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub fn chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
pub fn clean(s: &str, n: usize) -> String {
    let raw = s
        .chars()
        .take(n * 4)
        .filter(|c| !matches!(*c,'\u{0}'..='\u{8}'|'\u{b}'..='\u{1f}'|'\u{7f}'))
        .collect::<String>();
    chars(raw.trim(), n)
}
pub fn bus_name(s: &str) -> bool {
    if s.is_empty() || s.len() > 255 {
        return false;
    }
    // Python's original anchored regex accepts one final LF; registration trims it.
    let s = s.strip_suffix('\n').unwrap_or(s);
    let (unique, s) = if let Some(s) = s.strip_prefix(':') {
        (true, s)
    } else {
        (false, s)
    };
    let parts = s.split('.').collect::<Vec<_>>();
    parts.len() > 1
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_alphabetic()
                        || b == b'_'
                        || b == b'-'
                        || b.is_ascii_digit() && (unique || i > 0)
                })
        })
}
pub fn object_path(s: &str) -> bool {
    let len = s.len();
    let s = s.strip_suffix('\n').unwrap_or(s);
    len > 0
        && len <= 1024
        && (s == "/"
            || s.starts_with('/')
                && s[1..].split('/').all(|p| {
                    !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                }))
}

pub fn registration(service: &str, sender: Option<&str>) -> Option<(String, String)> {
    if service.chars().count() > 1280 {
        return None;
    }
    let s = service.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('/') {
        let sender = sender?;
        return (bus_name(sender) && object_path(s)).then(|| (sender.into(), s.into()));
    }
    let (name, path) = if let Some((name, path)) = s.split_once('@') {
        (name, path.to_string())
    } else if let Some((name, rest)) = s.split_once('/') {
        (name, format!("/{rest}"))
    } else {
        (s, ITEM_PATH.into())
    };
    (bus_name(name) && object_path(&path)).then(|| (name.into(), path))
}
pub fn generic(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    s.is_empty()
        || s.chars()
            .all(|c| unsafe { glib::ffi::g_unichar_isdigit(c as u32) != 0 })
        || ["chrome_status_icon_", "tray_icon_"].iter().any(|p| {
            s.strip_prefix(p).is_some_and(|r| {
                !r.is_empty()
                    && r.chars()
                        .all(|c| unsafe { glib::ffi::g_unichar_isdigit(c as u32) != 0 })
            })
        })
}
pub fn item_key(desktop: Option<&str>, id: &str, exe: Option<&str>) -> String {
    chars(
        &if let Some(s) = desktop {
            s.into()
        } else if !generic(id) {
            format!("sni:{}", id.trim())
        } else if let Some(exe) = exe {
            format!("exe:{exe}")
        } else {
            format!(
                "sni:{}",
                if id.trim().is_empty() {
                    "onbekend"
                } else {
                    id.trim()
                }
            )
        },
        200,
    )
}
pub fn status(s: &str) -> &'static str {
    match s.trim().to_lowercase().as_str() {
        "passive" => "passive",
        "needsattention" => "attention",
        _ => "active",
    }
}
pub fn mnemonic(s: &str) -> String {
    let mut chars = s.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_')
            }
        } else {
            out.push(c)
        }
    }
    out
}
pub fn shortcut(value: &Variant) -> Option<String> {
    let combos = value.get::<Vec<Vec<String>>>()?;
    let mut out = Vec::new();
    for combo in combos.into_iter().take(3) {
        let keys = combo
            .into_iter()
            .take(6)
            .filter(|s| !s.is_empty() && s.chars().count() <= 40)
            .map(|k| match k.to_lowercase().as_str() {
                "control" | "ctrl" => "Ctrl".into(),
                "alt" => "Alt".into(),
                "shift" => "Shift".into(),
                "super" => "Super".into(),
                "return" | "kp_enter" => "Enter".into(),
                "escape" => "Esc".into(),
                "delete" => "Del".into(),
                "backspace" => "Backspace".into(),
                "space" => "Space".into(),
                "page_up" => "PgUp".into(),
                "page_down" => "PgDn".into(),
                "plus" => "+".into(),
                "minus" => "-".into(),
                _ => {
                    if k.chars().count() == 1 {
                        k.to_uppercase()
                    } else {
                        k
                    }
                }
            })
            .collect::<Vec<String>>();
        if !keys.is_empty() {
            out.push(keys.join("+"))
        }
    }
    (!out.is_empty()).then(|| out.join(", "))
}
pub fn dict(v: &Variant) -> HashMap<String, Variant> {
    if v.type_().as_str() != "a{sv}" {
        return HashMap::new();
    }
    (0..v.n_children().min(200))
        .filter_map(|i| {
            let child = v.child_value(i);
            Some((
                child.child_value(0).str()?.to_owned(),
                child.child_value(1).as_variant()?,
            ))
        })
        .collect()
}
pub fn string(v: Option<&Variant>) -> Option<String> {
    v?.str().map(str::to_owned)
}
pub fn boolean(v: Option<&Variant>) -> Option<bool> {
    v?.get::<bool>()
}
pub fn integer(v: Option<&Variant>) -> Option<i64> {
    let v = v?;
    match v.type_().as_str() {
        "i" => v.get::<i32>().map(i64::from),
        "u" => v.get::<u32>().map(i64::from),
        "x" => v.get::<i64>(),
        "n" => v.get::<i16>().map(i64::from),
        "q" => v.get::<u16>().map(i64::from),
        "y" => v.get::<u8>().map(i64::from),
        _ => None,
    }
}
pub fn icon_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(c) = chars.next() else { return false };
    (c.is_ascii_alphanumeric() || "_+@-".contains(c))
        && s.chars().count() <= 200
        && chars.all(|c| c.is_ascii_alphanumeric() || "_.+@ -".contains(c))
}
// HTML5 named character references; table values are Unicode entity mappings.
fn unescape(s: &str) -> String {
    static ENTITIES: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    let entities = ENTITIES.get_or_init(|| {
        serde_json::from_str(include_str!("html-entities.json")).expect("HTML entities")
    });
    let mut out = String::new();
    let mut at = 0;
    while at < s.len() {
        let rest = &s[at..];
        if !rest.starts_with('&') {
            let c = rest.chars().next().unwrap();
            out.push(c);
            at += c.len_utf8();
            continue;
        }
        let after = &rest[1..];
        if let Some(number) = after.strip_prefix('#') {
            let (hex, digits) = if let Some(d) = number
                .strip_prefix('x')
                .or_else(|| number.strip_prefix('X'))
            {
                (true, d)
            } else {
                (false, number)
            };
            let count = digits
                .bytes()
                .take_while(|b| {
                    if hex {
                        b.is_ascii_hexdigit()
                    } else {
                        b.is_ascii_digit()
                    }
                })
                .count();
            if count > 0 {
                let raw = &digits[..count];
                let n = u32::from_str_radix(raw, if hex { 16 } else { 10 }).unwrap_or(0x110000);
                let n = match n {
                    0 => 0xfffd,
                    128 => 0x20ac,
                    130 => 0x201a,
                    131 => 0x192,
                    132 => 0x201e,
                    133 => 0x2026,
                    134 => 0x2020,
                    135 => 0x2021,
                    136 => 0x2c6,
                    137 => 0x2030,
                    138 => 0x160,
                    139 => 0x2039,
                    140 => 0x152,
                    142 => 0x17d,
                    145 => 0x2018,
                    146 => 0x2019,
                    147 => 0x201c,
                    148 => 0x201d,
                    149 => 0x2022,
                    150 => 0x2013,
                    151 => 0x2014,
                    152 => 0x2dc,
                    153 => 0x2122,
                    154 => 0x161,
                    155 => 0x203a,
                    156 => 0x153,
                    158 => 0x17e,
                    159 => 0x178,
                    _ => n,
                };
                let invalid = n == 11
                    || (1..=8).contains(&n)
                    || (14..=31).contains(&n)
                    || (0x7f..=0x9f).contains(&n) && ![129, 141, 143, 144, 157].contains(&n)
                    || (0xfdd0..=0xfdef).contains(&n)
                    || n <= 0x10ffff && n & 0xffff >= 0xfffe;
                if !invalid {
                    out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                }
                at += 1
                    + 1
                    + usize::from(hex)
                    + count
                    + usize::from(digits[count..].starts_with(';'));
                continue;
            }
        }
        let mut matched = None;
        let mut ends = after
            .char_indices()
            .take(34)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        ends.push(after.len().min(34));
        for end in ends.into_iter().rev() {
            if let Some(part) = after.get(..end) {
                if let Some(value) = entities.get(part) {
                    matched = Some((end, value));
                    break;
                }
            }
        }
        if let Some((end, value)) = matched {
            out.push_str(value);
            at += 1 + end
        } else {
            out.push('&');
            at += 1
        }
    }
    out
}
pub fn markup(s: &str, n: usize) -> String {
    let s = chars(s, n * 4);
    let mut text = String::new();
    let mut at = 0;
    while at < s.len() {
        let rest = &s[at..];
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                if rest[..end].chars().count() <= 501 {
                    let tag = rest[1..end].trim().trim_end_matches('/').trim();
                    if tag.eq_ignore_ascii_case("br") {
                        text.push('\n')
                    }
                    at += end + 1;
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap();
        text.push(c);
        at += c.len_utf8()
    }
    let text = unescape(&text);
    let mut normalized = String::new();
    for c in text.chars() {
        if c == '\n' {
            while normalized.ends_with([' ', '\t']) {
                normalized.pop();
            }
        }
        normalized.push(c);
    }
    let text = normalized;
    chars(text.trim(), n)
}
pub fn theme_icon(theme: &str, name: &str) -> Option<PathBuf> {
    if !Path::new(theme).is_absolute()
        || theme.contains('\0')
        || !icon_name(name)
        || name.starts_with('.')
    {
        return None;
    }
    let base = fs::canonicalize(theme).ok()?;
    if !base.is_dir() {
        return None;
    }
    let mut candidates = vec![
        base.join(format!("{name}.png")),
        base.join(format!("{name}.svg")),
    ];
    if let Ok(entries) = fs::read_dir(base.join("hicolor")) {
        let mut sizes = entries
            .take(64)
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        sizes.sort_by_key(|s| {
            if s == "scalable" {
                (0, 0)
            } else if let Ok(n) = s.split(['x', '@']).next().unwrap_or("").parse::<i64>() {
                (1, -n)
            } else {
                (2, 0)
            }
        });
        for size in sizes {
            for sub in ["apps", "status"] {
                for ext in ["png", "svg"] {
                    candidates.push(
                        base.join("hicolor")
                            .join(&size)
                            .join(sub)
                            .join(format!("{name}.{ext}")),
                    );
                }
            }
        }
    }
    for p in candidates {
        if let Ok(real) = fs::canonicalize(p) {
            if real.starts_with(&base)
                && real != base
                && real.is_file()
                && ["png", "svg"].contains(
                    &real
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .as_str(),
                )
            {
                return Some(real);
            }
        }
    }
    None
}
pub fn pyint(v: &Value, default: i64) -> Option<i64> {
    match v {
        Value::Null => Some(default),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_f64().map(|n| n as i64),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}
pub fn error_name(e: &glib::Error) -> String {
    use glib::translate::ToGlibPtr;
    let ptr = e.to_glib_none().0;
    let remote = unsafe { gio::ffi::g_dbus_error_get_remote_error(ptr) };
    if !remote.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(remote) }
            .to_string_lossy()
            .into_owned();
        unsafe { glib::ffi::g_free(remote.cast()) };
        return s;
    }
    let raw = unsafe { &*ptr };

    if e.matches(gio::IOErrorEnum::TimedOut)
        || raw.domain == unsafe { gio::ffi::g_dbus_error_quark() }
            && [
                gio::ffi::G_DBUS_ERROR_TIMEOUT,
                gio::ffi::G_DBUS_ERROR_NO_REPLY,
            ]
            .contains(&raw.code)
    {
        "timeout"
    } else if e.matches(gio::IOErrorEnum::Cancelled) {
        "cancelled"
    } else if e.matches(gio::IOErrorEnum::Closed) {
        "closed"
    } else {
        return format!("{}:{}", e.domain().as_str(), raw.code);
    }
    .into()
}
pub fn error_kind(e: &glib::Error) -> &'static str {
    let name = error_name(e);
    let message = e.message().to_lowercase();
    if name == "cancelled" {
        "cancelled"
    } else if [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.Disconnected",
        "org.freedesktop.DBus.Error.NoReply",
        "closed",
    ]
    .contains(&name.as_str())
    {
        "gone"
    } else if [
        "timeout",
        "org.freedesktop.DBus.Error.Timeout",
        "org.freedesktop.DBus.Error.TimedOut",
    ]
    .contains(&name.as_str())
    {
        "timeout"
    } else if name == "org.freedesktop.DBus.Error.UnknownObject"
        || name == "org.freedesktop.DBus.Error.UnknownMethod"
            && message.contains("object does not exist")
    {
        "noobject"
    } else if [".UnknownMethod", ".NotSupported", ".UnknownInterface"]
        .iter()
        .any(|s| name.ends_with(s))
        || name == "org.freedesktop.DBus.Error.Failed"
            && [
                "not supported",
                "not implemented",
                "unsupported",
                "unknown method",
                "no such method",
                "not available",
            ]
            .iter()
            .any(|s| message.contains(s))
    {
        "unsupported"
    } else {
        "other"
    }
}
pub fn menu(
    layout: &Variant,
    write_icon: &dyn Fn(&[u8]) -> Option<String>,
    parents: &mut Vec<i32>,
) -> Vec<Value> {
    fn node(
        v: &Variant,
        depth: usize,
        budget: &mut usize,
        write: &dyn Fn(&[u8]) -> Option<String>,
        parents: &mut Vec<i32>,
    ) -> Option<Value> {
        if v.type_().as_str() != "(ia{sv}av)" || *budget == 0 {
            return None;
        }
        *budget -= 1;
        let id = v.child_value(0).get::<i32>()?;
        let props = dict(&v.child_value(1));
        if boolean(props.get("visible")) == Some(false) {
            return None;
        }
        let sep = string(props.get("type")).as_deref() == Some("separator");
        let mut icon = None;
        if !sep {
            if let Some(data) = props
                .get("icon-data")
                .filter(|v| v.type_().as_str() == "ay")
            {
                if let Ok(data) = data.fixed_array::<u8>() {
                    if data.len() <= 512 * 1024 {
                        icon = write(data)
                    }
                }
            }
            if icon.is_none() {
                if let Some(name) = string(props.get("icon-name")) {
                    if icon_name(&name)
                        || name.starts_with('/')
                            && [".png", ".svg"]
                                .iter()
                                .any(|s| name.to_lowercase().ends_with(s))
                    {
                        icon = Some(name)
                    }
                }
            }
        }
        let toggle = string(props.get("toggle-type")).filter(|s| s == "checkmark" || s == "radio");
        let state = integer(props.get("toggle-state"));
        let kids = v.child_value(2);
        let submenu = string(props.get("children-display")).as_deref() == Some("submenu");
        if depth > 0 && !sep && (submenu || kids.n_children() > 0) && parents.len() < 100 {
            parents.push(id)
        }
        let mut children = Vec::new();
        if !sep && depth < 8 && (submenu || kids.n_children() > 0) {
            for i in 0..kids.n_children().min(500) {
                if *budget == 0 {
                    break;
                }
                if let Some(v) = kids.child_value(i).as_variant() {
                    if let Some(child) = node(&v, depth + 1, budget, write, parents) {
                        children.push(child)
                    }
                }
            }
            children = tidy(children)
        }
        let shortcut = props.get("shortcut").and_then(shortcut);
        let label = if sep {
            String::new()
        } else {
            chars(
                &mnemonic(&clean(&string(props.get("label")).unwrap_or_default(), 400)),
                200,
            )
        };
        Some(
            json!({"id":id,"type":if sep{"separator"}else{"item"},"label":label,"enabled":boolean(props.get("enabled")).unwrap_or(true),"icon":icon,"toggle":if sep{None}else{toggle},"checked":if sep{None}else{state.filter(|s|*s==0||*s==1).map(|s|s==1)},"shortcut":if sep{None}else{shortcut},"submenu":!sep&&(submenu||!children.is_empty()),"children":children}),
        )
    }
    node(layout, 0, &mut 500, write_icon, parents)
        .and_then(|v| v["children"].as_array().cloned())
        .unwrap_or_default()
}
fn tidy(v: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for item in v {
        if item["type"] == "separator"
            && out.last().is_none_or(|i: &Value| i["type"] == "separator")
        {
            continue;
        }
        out.push(item)
    }
    while out.last().is_some_and(|i| i["type"] == "separator") {
        out.pop();
    }
    out
}

pub fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
