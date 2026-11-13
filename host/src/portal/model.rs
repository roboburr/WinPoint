//! Typed backend FileChooser options and result normalization. No file is created or opened for writing.
use gio::prelude::*;
use glib::{
    Variant,
    variant::{FromVariant, ObjectPath, ToVariant},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
pub(super) type Filter = (String, Vec<(u32, String)>);
pub(super) type Choice = (String, String, Vec<(String, String)>, String);
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    Open,
    Save,
    SaveMany,
}
#[derive(Clone, Debug)]
pub(super) struct Request {
    pub handle: String,
    pub app_id: String,
    pub parent: String,
    pub title: String,
    pub kind: Kind,
    pub multiple: bool,
    pub directory: bool,
    pub modal: bool,
    pub label: Option<String>,
    pub folder: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub name: Option<String>,
    pub files: Vec<Vec<u8>>,
    pub filters: Vec<Filter>,
    pub current: usize,
    pub choices: Vec<Choice>,
}
fn bad(what: &str) -> String {
    format!("Invalid FileChooser {what}")
}
fn option<T: FromVariant>(
    values: &BTreeMap<String, Variant>,
    key: &str,
) -> Result<Option<T>, String> {
    values
        .get(key)
        .map(|v| T::from_variant(v).ok_or_else(|| bad(key)))
        .transpose()
}
fn text(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !value.contains('\0')
}
fn bytes(value: Vec<u8>) -> Result<Vec<u8>, String> {
    if value.last() != Some(&0) || value.len() > 8193 {
        return Err(bad("filesystem byte string"));
    }
    let raw = value[..value.len() - 1].to_vec();
    if raw.contains(&0) {
        return Err(bad("filesystem byte string"));
    }
    Ok(raw)
}
fn path(values: &BTreeMap<String, Variant>, key: &str) -> Result<Option<PathBuf>, String> {
    option::<Vec<u8>>(values, key)?
        .map(|v| bytes(v).map(|b| PathBuf::from(OsString::from_vec(b))))
        .transpose()
        .map(|v| v.filter(|p| p.is_absolute()))
}
fn valid_filter(filter: &Filter) -> bool {
    text(&filter.0, 256)
        && filter.1.len() <= 128
        && filter
            .1
            .iter()
            .all(|(ty, value)| *ty <= 1 && !value.is_empty() && text(value, 4096))
}
impl Request {
    pub fn parse(method: &str, params: &Variant) -> Result<Self, String> {
        if params.size() > 2 * 1024 * 1024 {
            return Err(bad("option size"));
        }
        let (handle, app_id, parent, title, values) = params
            .get::<(
                ObjectPath,
                String,
                String,
                String,
                BTreeMap<String, Variant>,
            )>()
            .ok_or_else(|| bad("signature"))?;
        let handle = handle.to_string();
        if !handle.starts_with("/org/freedesktop/portal/desktop/request/") || handle.len() > 512 {
            return Err(bad("request handle"));
        }
        if !text(&app_id, 255) || !text(&parent, 1024) || !text(&title, 2048) {
            return Err(bad("title, app ID or parent"));
        }
        let kind = match method {
            "OpenFile" => Kind::Open,
            "SaveFile" => Kind::Save,
            "SaveFiles" => Kind::SaveMany,
            _ => return Err(bad("method")),
        };
        let multiple = kind == Kind::Open && option::<bool>(&values, "multiple")?.unwrap_or(false);
        let directory = kind == Kind::SaveMany
            || (kind == Kind::Open && option::<bool>(&values, "directory")?.unwrap_or(false));
        let modal = option::<bool>(&values, "modal")?.unwrap_or(true);
        let label = option::<String>(&values, "accept_label")?.filter(|s| !s.is_empty());
        if label.as_ref().is_some_and(|s| !text(s, 256)) {
            return Err(bad("accept_label"));
        }
        let folder = path(&values, "current_folder")?;
        let file = path(&values, "current_file")?;
        let name = option::<String>(&values, "current_name")?;
        if name
            .as_ref()
            .is_some_and(|s| !text(s, 4096) || s.contains('/'))
        {
            return Err(bad("current_name"));
        }
        let mut filters = option::<Vec<Filter>>(&values, "filters")?.unwrap_or_default();
        if filters.len() > 32 || filters.iter().any(|f| !valid_filter(f)) {
            return Err(bad("filters"));
        }
        let current_filter = option::<Filter>(&values, "current_filter")?;
        if current_filter.as_ref().is_some_and(|f| !valid_filter(f)) {
            return Err(bad("current_filter"));
        }
        if filters.is_empty() {
            if let Some(filter) = current_filter.clone() {
                filters.push(filter);
            }
        }
        let current = current_filter
            .and_then(|f| {
                filters
                    .iter()
                    .position(|candidate| *candidate == f)
                    .or_else(|| filters.iter().position(|candidate| candidate.0 == f.0))
            })
            .unwrap_or(0);
        let mut choices = option::<Vec<Choice>>(&values, "choices")?.unwrap_or_default();
        let mut ids = BTreeSet::new();
        if choices.len() > 32 {
            return Err(bad("choices"));
        }
        for (id, label, items, selected) in &mut choices {
            if id.is_empty()
                || label.is_empty()
                || !text(id, 256)
                || !text(label, 1024)
                || !ids.insert(id.clone())
                || items.len() > 256
            {
                return Err(bad("choices"));
            }
            let mut item_ids = BTreeSet::new();
            for (key, label) in items.iter() {
                if key.is_empty()
                    || label.is_empty()
                    || !text(key, 256)
                    || !text(label, 1024)
                    || !item_ids.insert(key)
                {
                    return Err(bad("choice items"));
                }
            }
            if items.is_empty() {
                *selected = if selected == "true" { "true" } else { "false" }.into();
            } else if !items.iter().any(|(id, _)| id == selected) {
                *selected = items[0].0.clone();
            }
        }
        let mut files = vec![];
        if kind == Kind::SaveMany {
            let names = option::<Vec<Vec<u8>>>(&values, "files")?.unwrap_or_default();
            if names.is_empty() || names.len() > 1000 {
                return Err(bad("files"));
            }
            for value in names {
                let value = bytes(value)?;
                if value.is_empty()
                    || value.len() > 255
                    || value.contains(&b'/')
                    || value == b"."
                    || value == b".."
                {
                    return Err(bad("file name"));
                }
                files.push(value);
            }
        }
        Ok(Self {
            handle,
            app_id,
            parent,
            title,
            kind,
            multiple,
            directory,
            modal,
            label,
            folder,
            file,
            name,
            files,
            filters,
            current,
            choices,
        })
    }
    pub fn picker(&self) -> Value {
        let folder = self
            .file
            .as_ref()
            .and_then(|f| f.parent().map(Path::to_path_buf))
            .or_else(|| self.folder.clone())
            .filter(|p| p.is_dir())
            .and_then(|mut p| {
                while p.to_str().is_none() {
                    if !p.pop() {
                        return None;
                    }
                }
                p.to_str().map(str::to_owned)
            });
        let name = self
            .name
            .clone()
            .or_else(|| {
                self.file
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .and_then(|n| n.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        json!({"portal":true,"save":self.kind==Kind::Save,"folder":self.directory,"multiple":self.multiple,"modal":self.modal,"title":self.title,
            "directory":folder,"name":name,"acceptLabel":self.label.as_deref().map(mnemonic),"filters":self.filters.iter().map(|(name,rules)|json!({"name":name,"patterns":rules.iter().map(|(_,p)|p).collect::<Vec<_>>(),"conditions":rules.iter().map(|(kind,value)|json!({"type":kind,"value":value})).collect::<Vec<_>>(),"caseSensitive":true})).collect::<Vec<_>>(),"currentFilter":self.current,
            "choices":self.choices.iter().map(|(id,label,items,selected)|json!({"id":id,"label":label,"options":items.iter().map(|(id,label)|json!({"id":id,"label":label})).collect::<Vec<_>>(),"selected":selected})).collect::<Vec<_>>(),"allowReadOnly":self.kind==Kind::Open&&!self.directory,"writable":true})
    }
    pub fn result(&self, value: &Value) -> Result<Option<BTreeMap<String, Variant>>, String> {
        let paths = if let Some(values) = value["paths"].as_array() {
            if values.len() > 1000 {
                return Err(bad("selection"));
            }
            values
                .iter()
                .map(|v| {
                    v.as_str()
                        .filter(|p| !p.is_empty() && text(p, 8192))
                        .map(PathBuf::from)
                        .ok_or_else(|| bad("selection"))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else if let Some(path) = value["path"].as_str().filter(|s| !s.is_empty()) {
            vec![PathBuf::from(path)]
        } else {
            return Ok(None);
        };
        if paths.is_empty() {
            return Ok(None);
        }
        if !self.multiple && paths.len() != 1 {
            return Err(bad("selection count"));
        }
        for path in &paths {
            if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
                return Err(bad("selection path"));
            }
            let valid = if self.directory {
                path.is_dir()
            } else if self.kind == Kind::Save {
                !path.is_dir() && path.parent().is_some_and(Path::is_dir)
            } else {
                path.is_file()
            };
            if !valid {
                return Err(bad("selection path"));
            }
        }
        let paths = if self.kind == Kind::SaveMany {
            unique(&paths[0], &self.files)?
        } else {
            paths
        };
        let uris = paths
            .iter()
            .map(|p| gio::File::for_path(p).uri().to_string())
            .collect::<Vec<_>>();
        let mut results = BTreeMap::from([
            ("uris".into(), uris.to_variant()),
            (
                "writable".into(),
                value["writable"].as_bool().unwrap_or(true).to_variant(),
            ),
        ]);
        if !self.filters.is_empty() {
            let index = value["filterIndex"]
                .as_u64()
                .map(|i| i as usize)
                .unwrap_or(self.current);
            let filter = self
                .filters
                .get(index)
                .ok_or_else(|| bad("selected filter"))?;
            results.insert("current_filter".into(), filter.to_variant());
        }
        let mut selected = BTreeMap::new();
        if let Some(rows) = value["choices"].as_array() {
            for row in rows {
                let pair = row
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .ok_or_else(|| bad("selected choices"))?;
                let (id, choice) = (
                    pair[0].as_str().ok_or_else(|| bad("selected choices"))?,
                    pair[1].as_str().ok_or_else(|| bad("selected choices"))?,
                );
                if selected.insert(id.to_owned(), choice.to_owned()).is_some() {
                    return Err(bad("selected choices"));
                }
            }
        }
        let mut choices = vec![];
        for (id, _, items, default) in &self.choices {
            let choice = selected.remove(id).unwrap_or_else(|| default.clone());
            if (items.is_empty() && !matches!(choice.as_str(), "true" | "false"))
                || (!items.is_empty() && !items.iter().any(|(key, _)| key == &choice))
            {
                return Err(bad("selected choices"));
            }
            choices.push((id.clone(), choice));
        }
        if !selected.is_empty() {
            return Err(bad("selected choices"));
        }
        results.insert("choices".into(), choices.to_variant());
        Ok(Some(results))
    }
}
fn mnemonic(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            if let Some(next) = chars.next() {
                result.push(next);
            }
        } else {
            result.push(c);
        }
    }
    result
}
fn unique(folder: &Path, names: &[Vec<u8>]) -> Result<Vec<PathBuf>, String> {
    let mut result = vec![];
    let mut reserved = BTreeSet::new();
    for name in names {
        let raw = OsString::from_vec(name.clone());
        let original = folder.join(&raw);
        let mut path = original.clone();
        let mut counter = 0;
        loop {
            let exists = match std::fs::symlink_metadata(&path) {
                Ok(_) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err(bad("save directory")),
            };
            if !exists && !reserved.contains(&path) {
                break;
            }
            counter += 1;
            if counter > 10000 {
                return Err(bad("save name collision"));
            }
            let basename = Path::new(&raw);
            let stem = basename.file_stem().unwrap_or(&raw);
            let ext = basename.extension();
            let mut next = stem.as_bytes().to_vec();
            next.extend(format!(" ({counter})").as_bytes());
            if let Some(ext) = ext {
                next.push(b'.');
                next.extend(ext.as_bytes());
            }
            path = folder.join(OsString::from_vec(next));
        }
        reserved.insert(path.clone());
        result.push(path);
    }
    Ok(result)
}
