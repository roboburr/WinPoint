//! Paths and URIs handed to our Explorer, grouped into the original tabs/selection format.
use gio::prelude::*;
use gtk::gio;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
};

pub(crate) fn explorer(uris: &[String]) -> Value {
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    let mut by_folder = HashMap::new();
    for uri in uris.iter().take(2000) {
        let Some((place, mut path)) = classify(uri) else {
            continue;
        };
        let mut selection = None;
        if !place {
            let p = Path::new(&path);
            if !p.is_dir() {
                let parent = p.parent().unwrap_or(Path::new("/"));
                let exists = std::fs::symlink_metadata(p).is_ok();
                let folder = nearest_dir(parent);
                if exists && folder == parent && p != Path::new("/") {
                    selection = Some(path.clone());
                }
                path = folder.to_string_lossy().into_owned();
            }
        }
        let index = *by_folder.entry(path.clone()).or_insert_with(|| {
            let index = entries.len();
            entries.push((path, Vec::new()));
            index
        });
        if let Some(selection) = selection
            && !entries[index].1.contains(&selection)
        {
            entries[index].1.push(selection);
        }
    }
    entries.truncate(20);
    for (_, selection) in &mut entries {
        selection.truncate(500);
    }
    let mut result = build(&entries);
    while default_json_len(&result) > 48 * 1024 {
        if entries.len() > 1 {
            entries.pop();
        } else if let Some((_, selection)) = entries.first_mut() {
            if selection.is_empty() {
                break;
            }
            selection.truncate(selection.len() / 2);
        } else {
            break;
        }
        result = build(&entries);
    }
    result
}
fn build(entries: &[(String, Vec<String>)]) -> Value {
    let one = |(path, selected): &(String, Vec<String>)| {
        let mut value = json!({"path":path});
        if !selected.is_empty() {
            value["select"] = json!(selected);
        }
        value
    };
    let Some(first) = entries.first() else {
        return json!({});
    };
    let mut result = one(first);
    if entries.len() > 1 {
        result["tabs"] = Value::Array(entries.iter().skip(1).map(one).collect());
    }
    result
}
fn default_json_len(value: &Value) -> usize {
    match value {
        Value::Array(a) => {
            2 + a.iter().map(default_json_len).sum::<usize>() + a.len().saturating_sub(1) * 2
        }
        Value::Object(o) => {
            2 + o
                .iter()
                .map(|(k, v)| serde_json::to_string(k).unwrap().len() + 2 + default_json_len(v))
                .sum::<usize>()
                + o.len().saturating_sub(1) * 2
        }
        _ => serde_json::to_string(value).unwrap().len(),
    }
}
fn classify(uri: &str) -> Option<(bool, String)> {
    let uri = if uri.starts_with('/') {
        uri
    } else {
        uri.trim()
    };
    if uri.is_empty()
        || uri.contains('\0')
        || uri.chars().any(|c| c < ' ')
        || uri.chars().count() > 8192
    {
        return None;
    }
    let scheme = uri
        .split_once(':')
        .map(|(s, _)| s.to_lowercase())
        .unwrap_or_default();
    if matches!(scheme.as_str(), "trash" | "computer" | "recent") {
        return Some((true, format!("{scheme}:///")));
    }
    let path = if uri.starts_with('/') {
        PathBuf::from(uri)
    } else if scheme == "file" {
        let url = url::Url::parse(uri).ok()?;
        let host = url.host_str().unwrap_or("");
        let mut hostname = [0_i8; 256];
        unsafe {
            libc::gethostname(hostname.as_mut_ptr(), hostname.len());
        }
        let own = unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }
            .to_string_lossy()
            .to_lowercase();
        if !host.is_empty() && host != "localhost" && host != own {
            return None;
        }
        let mut raw = Vec::new();
        let bytes = url.path().as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%'
                && at + 2 < bytes.len()
                && let Ok(byte) = u8::from_str_radix(&url.path()[at + 1..at + 3], 16)
            {
                raw.push(byte);
                at += 3;
            } else {
                raw.push(bytes[at]);
                at += 1;
            }
        }
        if raw.contains(&0) {
            return None;
        }
        PathBuf::from(std::ffi::OsString::from_vec(raw))
    } else {
        gio::File::for_uri(uri).path()?
    };
    let mut path = normalize(&path);
    if path.to_str().is_none() {
        while path.to_str().is_none() {
            path = path.parent()?.to_path_buf();
        }
        path = nearest_dir(&path);
    }
    Some((false, path.to_str()?.to_owned()))
}
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out
}
fn nearest_dir(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    while !path.is_dir() {
        if !path.pop() {
            return PathBuf::from("/");
        }
    }
    path
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_share_selection_and_folders_get_separate_tabs() {
        let root =
            std::env::temp_dir().join(format!("win11-app-params-{}", crate::events::nonce()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("A")).unwrap();
        std::fs::create_dir(root.join("B")).unwrap();
        std::fs::write(root.join("A/one"), "").unwrap();
        std::fs::write(root.join("A/two"), "").unwrap();
        let value = explorer(&[
            root.join("A/one").to_string_lossy().into_owned(),
            root.join("B").to_string_lossy().into_owned(),
            root.join("A/two").to_string_lossy().into_owned(),
        ]);
        assert_eq!(value["path"], json!(root.join("A")));
        assert_eq!(
            value["select"],
            json!([root.join("A/one"), root.join("A/two")])
        );
        assert_eq!(value["tabs"], json!([{"path":root.join("B")} ]));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_inputs_special_places_and_other_hosts() {
        let root = std::env::temp_dir();
        assert_eq!(
            explorer(&[format!(
                "{}/not-present-{}/child",
                root.display(),
                crate::events::nonce()
            )])["path"],
            json!(root)
        );
        assert_eq!(
            explorer(&["trash:///".into(), "computer:///".into()]),
            json!({"path":"trash:///","tabs":[{"path":"computer:///"}]})
        );
        assert_eq!(
            explorer(&[
                "https://example.invalid".into(),
                "file://other.invalid/tmp".into()
            ]),
            json!({})
        );
    }
}
