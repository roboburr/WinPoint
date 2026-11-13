//! GNOME lacks the data-control protocol used by the reference history service.
//! Never start wl-paste or a focus-stealing helper here; retain pinned data and the same unsupported API.
use super::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
pub(super) struct Clipboard {
    _history: History,
    pub reason: String,
}
impl Clipboard {
    pub fn new(ctx: &ServiceContext) -> Self {
        let reason = if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|v| v.is_empty()) {
            "no-wayland"
        } else if super::super::storage::common::which("wl-paste", None).is_none()
            || super::super::storage::common::which("wl-copy", None).is_none()
        {
            "no-wl-clipboard"
        } else {
            "gnome"
        };
        Self {
            _history: History::new(ctx.config_dir.join("clipboard-pinned.json")),
            reason: reason.into(),
        }
    }
    pub fn call(&self, method: &str, _args: &Value) -> Option<ApiResult> {
        Some(Ok(match method {
            "clipboard.list" => json!({"supported":false,"items":[]}),
            "clipboard.paste" | "clipboard.remove" | "clipboard.clear" | "clipboard.pin" => {
                json!({"supported":false})
            }
            _ => return None,
        }))
    }
}
#[allow(dead_code)]
pub(super) struct History {
    path: PathBuf,
    items: Vec<Value>,
}
#[allow(dead_code)]
impl History {
    pub fn new(path: PathBuf) -> Self {
        let mut items = vec![];
        let loaded = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or(Value::Null);
        for mut item in loaded.as_array().into_iter().flatten().take(25).cloned() {
            if !item.is_object()
                || !matches!(item["kind"].as_str(), Some("text" | "image"))
                || !truth(&item["id"])
            {
                continue;
            }
            item["pinned"] = json!(true);
            let raw = if item["kind"] == "text" {
                string(&item["text"]).into_bytes()
            } else {
                let data = if truth(&item["data"]) {
                    string(&item["data"])
                } else {
                    string(&item["preview"])
                };
                let Some(raw) = decode(data.split_once(',').map(|(_, d)| d).unwrap_or("")) else {
                    continue;
                };
                raw
            };
            item["digest"] = json!(digest(&string(&item["kind"]), &raw));
            items.push(item);
        }
        Self { path, items }
    }
    pub fn public(&self) -> Value {
        json!(
            self.items
                .iter()
                .map(|i| {
                    let mut i = i.clone();
                    if let Some(m) = i.as_object_mut() {
                        for k in ["digest", "raw", "data"] {
                            m.remove(k);
                        }
                    }
                    i
                })
                .collect::<Vec<_>>()
        )
    }
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.items.iter().find(|i| i["id"] == id)
    }
    fn save(&self) {
        let pinned = self
            .items
            .iter()
            .filter(|i| truth(&i["pinned"]))
            .collect::<Vec<_>>();
        let _ =
            super::super::storage::common::atomic(&self.path, json!(pinned).to_string().as_bytes());
    }
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(i) = self.items.iter().position(|i| i["id"] == id) else {
            return false;
        };
        let item = self.items.remove(i);
        if truth(&item["pinned"]) {
            self.save();
        }
        true
    }
    pub fn clear(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|i| truth(&i["pinned"]));
        before - self.items.len()
    }
    pub fn pin(&mut self, id: &str, pinned: bool, lang: Lang) -> ApiResult {
        let Some(index) = self.items.iter().position(|i| i["id"] == id) else {
            return Err(ApiError::tr(
                "not_found",
                lang,
                "clipboard.notFound",
                crate::i18n::NO_PARAMS,
            ));
        };
        if pinned
            && !truth(&self.items[index]["pinned"])
            && self.items.iter().filter(|i| truth(&i["pinned"])).count() >= 25
        {
            return Err(ApiError::tr(
                "too_many",
                lang,
                "clipboard.pinLimit",
                crate::i18n::NO_PARAMS,
            ));
        }
        self.items[index]["pinned"] = json!(pinned);
        self.save();
        Ok(self.items[index].clone())
    }
    pub fn add(&mut self, kind: &str, text: Option<&str>, bytes: &[u8], mime: &str) -> Value {
        let raw = if kind == "text" {
            text.unwrap_or("").as_bytes()
        } else {
            bytes
        };
        let hash = digest(kind, raw);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if let Some(index) = self.items.iter().position(|i| i["digest"] == hash) {
            let mut item = self.items.remove(index);
            item["time"] = json!(now);
            self.items.insert(0, item.clone());
            return item;
        }
        let id = glib::uuid_string_random().replace('-', "")[..10].to_owned();
        let mut item =
            json!({"id":id,"kind":kind,"time":now,"pinned":false,"digest":hash,"size":raw.len()});
        if kind == "text" {
            item["text"] = json!(text);
        } else {
            let full = format!("data:{mime};base64,{}", glib::base64_encode(raw));
            let small = (raw.len() > 96 * 1024)
                .then(|| preview(raw, mime))
                .flatten();
            if let Some(small) = small.filter(|s| s.len() < full.len()) {
                item["preview"] = json!(small);
                item["data"] = json!(full);
            } else {
                item["preview"] = json!(full);
            }
            item["mime"] = json!(mime);
        }
        self.items.insert(0, item.clone());
        let mut keep = 0;
        self.items.retain(|i| {
            if truth(&i["pinned"]) {
                true
            } else {
                keep += 1;
                keep <= 25
            }
        });
        item
    }
    pub fn raw(item: &Value) -> Option<(Vec<u8>, String)> {
        if item["kind"] == "text" {
            return Some((
                string(&item["text"]).into_bytes(),
                "text/plain;charset=utf-8".into(),
            ));
        }
        let value = if truth(&item["data"]) {
            string(&item["data"])
        } else {
            string(&item["preview"])
        };
        let (head, data) = value.split_once(',').unwrap_or((&value, ""));
        let mime = head
            .strip_prefix("data:")
            .map(|s| s.split(';').next().unwrap_or(""))
            .unwrap_or("image/png");
        Some((decode(data)?, mime.into()))
    }
}
fn digest(kind: &str, data: &[u8]) -> String {
    format!(
        "{kind}:{}",
        glib::compute_checksum_for_data(glib::ChecksumType::Sha1, data).expect("SHA1 checksum")
    )
}
fn decode(value: &str) -> Option<Vec<u8>> {
    if !value.is_ascii() {
        return None;
    }
    let clean = value
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric() || [b'+', b'/', b'='].contains(b))
        .collect::<Vec<_>>();
    let n = clean.iter().take_while(|b| **b != b'=').count();
    let pad = clean.len() - n;
    if n % 4 == 1 || (n % 4 == 2 && pad < 2) || (n % 4 == 3 && pad < 1) {
        return None;
    }
    Some(glib::base64_decode(value))
}
fn preview(bytes: &[u8], mime: &str) -> Option<String> {
    use gtk::gdk_pixbuf::{self, prelude::*};
    if !["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime) {
        return None;
    }
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let mut pb = loader.pixbuf()?;
    let scale = 1f64.min(480f64 / f64::from(pb.width().max(pb.height()).max(1)));
    if scale < 1. {
        pb = pb.scale_simple(
            ((pb.width() as f64 * scale) as i32).max(1),
            ((pb.height() as f64 * scale) as i32).max(1),
            gdk_pixbuf::InterpType::Bilinear,
        )?;
    }
    let (fmt, mime, options) = if pb.has_alpha() {
        ("png", "image/png", vec![])
    } else {
        ("jpeg", "image/jpeg", vec![("quality", "85")])
    };
    let data = pb.save_to_bufferv(fmt, &options).ok()?;
    Some(format!("data:{mime};base64,{}", glib::base64_encode(&data)))
}
