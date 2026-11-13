//! Nonmutating previews for native file APIs. Dangerous wrapper shapes match the reference.
use super::{
    ApiError, ApiResult, Context, abs_in, directory, io_error, mutate, paths, validate_name,
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::MetadataExt, path::Path};
pub(super) fn result(extra: Value) -> Value {
    let mut out = if extra.is_object() { extra } else { json!({}) };
    out["ok"] = json!(true);
    out["dryRun"] = json!(true);
    out
}
pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    Some(match method {
        "files.rename" => rename(args, ctx),
        "files.copy" | "files.move" => (|| {
            paths(&args["paths"], ctx.lang())?;
            directory(&args["dest"], ctx.lang())?;
            Ok(result(json!({"paths":[],"failed":[]})))
        })(),
        "files.paste" => Ok(result(json!({"paths":[],"failed":[]}))),
        "files.trash" => Ok(result(json!({"failed":[],"trashed":[]}))),
        "files.emptyTrash" => Ok(result(json!({"freed":0}))),
        "files.restoreTrash" => Ok(result(json!({"restored":[],"failed":[]}))),
        "files.compress" | "files.extract" => Ok(result(json!({"path":null}))),
        "files.trustLauncher" => Ok(result(json!({"trusted":true}))),
        "files.setAttributes" | "files.openTerminal" => Ok(result(json!({}))),
        // These two reference handlers were not marked dangerous; native dry mode closes that gap.
        "files.createFolder" => create(args, ctx, true),
        "files.createFile" => create(args, ctx, false),
        _ => return None,
    })
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64() != Some(0.),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
fn rename(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let path = abs_in(&args["path"], lang)?;
    let name = validate_name(&args["newName"], lang)?;
    if fs::symlink_metadata(&path).is_err() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.itemGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    let target = path.parent().unwrap_or(Path::new("/")).join(&name);
    if target != path && fs::symlink_metadata(&target).is_ok() {
        let same = fs::metadata(&path)
            .ok()
            .zip(fs::metadata(&target).ok())
            .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino());
        if !same {
            return Err(ApiError::tr(
                "exists",
                lang,
                "files.nameExists",
                &serde_json::json!({"name": name.to_string()}),
            ));
        }
    }
    Ok(result(json!({"path":target})))
}
fn create(args: &Value, ctx: &Context, folder: bool) -> ApiResult {
    let lang = ctx.lang();
    let dest = directory(&args["dir"], lang)?;
    let kind = if truthy(&args["kind"]) {
        args["kind"].as_str().unwrap_or("")
    } else {
        "text"
    };
    if !folder && !matches!(kind, "text" | "empty") {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.unknownKind",
            crate::i18n::NO_PARAMS,
        ));
    }
    let name = if truthy(&args["name"]) {
        validate_name(&args["name"], lang)?
    } else {
        match (folder, kind) {
            (true, _) => crate::i18n::tr(lang, "files.new.folder"),
            (_, "empty") => crate::i18n::tr(lang, "files.new.empty"),
            _ => crate::i18n::tr(lang, "files.new.text"),
        }
    };
    let target = mutate::unique_path(&dest, &name, folder).map_err(|e| io_error(e, &dest, lang))?;
    Ok(result(json!({"path":target,"item":null})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "win11-files-dry-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn ctx(&self, lang: &str) -> Context {
            Context::new(self.0.clone(), self.0.clone(), lang.into(), false)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn dangerous_previews_preserve_fixed_reference_shapes_without_validation() {
        let t = Temp::new();
        let c = t.ctx("en");
        for (method, extra) in [
            ("files.trash", json!({"trashed":[],"failed":[]})),
            ("files.restoreTrash", json!({"restored":[],"failed":[]})),
            ("files.emptyTrash", json!({"freed":0})),
            ("files.paste", json!({"paths":[],"failed":[]})),
            ("files.compress", json!({"path":null})),
            ("files.extract", json!({"path":null})),
            ("files.trustLauncher", json!({"trusted":true})),
            ("files.setAttributes", json!({})),
            ("files.openTerminal", json!({})),
        ] {
            assert_eq!(
                call(method, &json!({"invalid":"ignored"}), &c)
                    .unwrap()
                    .unwrap(),
                result(extra)
            );
        }
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
    }
    #[test]
    fn rename_preview_checks_conflicts_but_never_renames_including_same_file_links() {
        let t = Temp::new();
        let c = t.ctx("en");
        let source = t.0.join("source.txt");
        fs::write(&source, "original").unwrap();
        let r = call(
            "files.rename",
            &json!({"path":source,"newName":"target.txt"}),
            &c,
        )
        .unwrap()
        .unwrap();
        assert_eq!(r, result(json!({"path":t.0.join("target.txt")})));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original");
        assert!(!t.0.join("target.txt").exists());
        fs::write(t.0.join("conflict"), "keep").unwrap();
        assert_eq!(
            call(
                "files.rename",
                &json!({"path":source,"newName":"conflict"}),
                &c
            )
            .unwrap()
            .unwrap_err()
            .code,
            "exists"
        );
        fs::hard_link(&source, t.0.join("same.txt")).unwrap();
        assert!(
            call(
                "files.rename",
                &json!({"path":source,"newName":"same.txt"}),
                &c
            )
            .unwrap()
            .is_ok()
        );
        assert_eq!(
            fs::read_to_string(t.0.join("same.txt")).unwrap(),
            "original"
        );
        symlink("missing", t.0.join("broken")).unwrap();
        assert!(
            call(
                "files.rename",
                &json!({"path":t.0.join("broken"),"newName":"renamed"}),
                &c
            )
            .unwrap()
            .is_ok()
        );
        assert!(t.0.join("broken").is_symlink());
    }
    #[test]
    fn transfer_preview_validates_paths_and_destination_without_copying() {
        let t = Temp::new();
        let c = t.ctx("nl");
        fs::create_dir(t.0.join("dest")).unwrap();
        fs::write(t.0.join("source"), "data").unwrap();
        for method in ["files.copy", "files.move"] {
            assert_eq!(
                call(
                    method,
                    &json!({"paths":[t.0.join("source")],"dest":t.0.join("dest")}),
                    &c
                )
                .unwrap()
                .unwrap(),
                result(json!({"paths":[],"failed":[]}))
            );
            assert_eq!(
                call(method, &json!({"paths":[],"dest":t.0.join("dest")}), &c)
                    .unwrap()
                    .unwrap_err()
                    .code,
                "bad_args"
            );
            assert_eq!(
                call(
                    method,
                    &json!({"paths":[t.0.join("source")],"dest":t.0.join("missing")}),
                    &c
                )
                .unwrap()
                .unwrap_err()
                .code,
                "not_found"
            );
        }
        assert!(t.0.join("source").exists());
        assert_eq!(fs::read_dir(t.0.join("dest")).unwrap().count(), 0);
    }
    #[test]
    fn new_item_previews_use_real_unique_names_and_language_without_creation() {
        let t = Temp::new();
        fs::create_dir(t.0.join("Nieuwe map")).unwrap();
        fs::write(t.0.join("New Text Document.txt"), "keep").unwrap();
        for (method, lang, expected) in [
            ("files.createFolder", "nl", "Nieuwe map (2)"),
            ("files.createFile", "en", "New Text Document (2).txt"),
        ] {
            let r = call(method, &json!({"dir":t.0}), &t.ctx(lang))
                .unwrap()
                .unwrap();
            assert_eq!(r, result(json!({"path":t.0.join(expected),"item":null})));
            assert!(!t.0.join(expected).exists());
        }
        assert_eq!(
            call(
                "files.createFile",
                &json!({"dir":t.0,"kind":"bad"}),
                &t.ctx("en")
            )
            .unwrap()
            .unwrap_err()
            .code,
            "bad_args"
        );
        assert_eq!(
            call(
                "files.createFolder",
                &json!({"dir":t.0,"name":"../escape"}),
                &t.ctx("en")
            )
            .unwrap()
            .unwrap_err()
            .code,
            "bad_name"
        );
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 2);
    }
}
