//! Standard Explorer locations, using the original labels, ordering and themed icons.
use super::{ApiResult, Context, read::Factory, trash};
use gtk::glib;
use serde_json::{Value, json};
use std::{ffi::CStr, fs, path::PathBuf};

pub(super) fn call(method: &str, _args: &Value, ctx: &Context) -> Option<ApiResult> {
    (method == "files.places").then(|| Ok(locations(ctx, trash::count())))
}
fn locations(ctx: &Context, trash_count: usize) -> Value {
    let factory = Factory::new(ctx);
    let lang = ctx.lang();
    // Display names only: the folders on disk keep their own (XDG) names (D-G15).
    let mut rows = vec![
        (
            "home",
            account_name(lang),
            ctx.home.clone(),
            &["user-home"][..],
        ),
        (
            "desktop",
            crate::i18n::tr(lang, "files.place.desktop"),
            ctx.desktop.clone(),
            &["user-desktop"][..],
        ),
    ];
    for (id, name, kind, fallback, icons) in [
        (
            "documents",
            crate::i18n::tr(lang, "files.place.documents"),
            glib::UserDirectory::Documents,
            "Documents",
            &["folder-documents"][..],
        ),
        (
            "downloads",
            // the same word in Windows 11's Dutch and English
            crate::i18n::pick(lang, "Downloads", "Downloads").to_owned(),
            glib::UserDirectory::Downloads,
            "Downloads",
            &["folder-download", "folder-downloads"][..],
        ),
        (
            "music",
            crate::i18n::tr(lang, "files.place.music"),
            glib::UserDirectory::Music,
            "Music",
            &["folder-music"][..],
        ),
        (
            "pictures",
            crate::i18n::tr(lang, "files.place.pictures"),
            glib::UserDirectory::Pictures,
            "Pictures",
            &["folder-pictures"][..],
        ),
        (
            "videos",
            crate::i18n::tr(lang, "files.place.videos"),
            glib::UserDirectory::Videos,
            "Videos",
            &["folder-videos"][..],
        ),
    ] {
        let path = glib::user_special_dir(kind).unwrap_or_else(|| fallback_dir(ctx, fallback));
        if path != ctx.home && path.is_dir() {
            rows.push((id, name, path, icons));
        }
    }
    rows.push((
        "trash",
        crate::i18n::tr(lang, "files.place.trash"),
        PathBuf::from("trash:///"),
        if trash_count > 0 {
            &["user-trash-full", "user-trash"]
        } else {
            &["user-trash"]
        },
    ));
    rows.push((
        "root",
        crate::i18n::tr(lang, "files.place.root"),
        PathBuf::from("/"),
        &["drive-harddisk", "drive-harddisk-system"],
    ));
    Value::Array(rows.into_iter().map(|(id,name,path,icons)|json!({"id":id,"name":name,"path":if id=="trash"{"trash:///".to_owned()}else{path.to_string_lossy().into_owned()},"icon":factory.first_icon(icons,"folder"),"exists":id=="trash"||path.is_dir()})).collect())
}
fn fallback_dir(ctx: &Context, fallback: &str) -> PathBuf {
    let kind = match fallback {
        "Downloads" => "DOWNLOAD",
        other => {
            return parsed_dir(ctx, &other.to_uppercase())
                .unwrap_or_else(|| ctx.home.join(fallback));
        }
    };
    parsed_dir(ctx, kind).unwrap_or_else(|| ctx.home.join(fallback))
}
fn parsed_dir(ctx: &Context, kind: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(glib::user_config_dir().join("user-dirs.dirs")).ok()?;
    let key = format!("XDG_{kind}_DIR=");
    for line in text.lines() {
        if let Some(value) = line.trim().strip_prefix(&key) {
            let value = value
                .trim()
                .trim_matches('"')
                .replace("$HOME", &ctx.home.to_string_lossy());
            let path = if value == "~" {
                ctx.home.clone()
            } else if let Some(rest) = value.strip_prefix("~/") {
                ctx.home.join(rest)
            } else {
                PathBuf::from(value)
            };
            if path.is_absolute() && path != ctx.home {
                return Some(path);
            }
        }
    }
    None
}
fn account_name(lang: crate::i18n::Lang) -> String {
    let mut value = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0; 16384];
    if unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            value.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    } == 0
        && !result.is_null()
    {
        let value = unsafe { value.assume_init() };
        let field = |p: *const libc::c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        let real = field(value.pw_gecos)
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        if !real.is_empty() {
            return real;
        }
        return field(value.pw_name);
    }
    std::env::var("USER").unwrap_or_else(|_| crate::i18n::tr(lang, "files.place.user"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locations_keep_original_order_missing_desktop_and_trash_semantics() {
        let root = std::env::temp_dir().join(format!("win11-places-{}", crate::events::nonce()));
        fs::create_dir(&root).unwrap();
        let ctx = Context::new(
            root.clone(),
            root.join("missing-desktop"),
            "en".into(),
            false,
        );
        let value = locations(&ctx, 1);
        let rows = value.as_array().unwrap();
        assert_eq!(rows[0]["id"], "home");
        assert_eq!(rows[0]["exists"], true);
        assert_eq!(rows[1]["name"], "Desktop");
        assert_eq!(rows[1]["exists"], false);
        assert_eq!(rows[rows.len() - 2]["path"], "trash:///");
        assert_eq!(rows[rows.len() - 2]["exists"], true);
        assert_eq!(rows.last().unwrap()["id"], "root");
        fs::remove_dir_all(root).unwrap();
    }
}
