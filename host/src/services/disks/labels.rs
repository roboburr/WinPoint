use super::*;
use std::collections::BTreeSet;
pub(super) const OFFLINE_LABEL: &[&str] =
    &["ntfs", "exfat", "vfat", "btrfs", "xfs", "f2fs", "nilfs2"];
pub(super) fn label_rule(fs: &str) -> Option<(usize, &'static str)> {
    match fs {
        "ntfs" => Some((128, "chars")),
        "exfat" => Some((15, "chars")),
        "vfat" => Some((11, "fat")),
        "ext2" | "ext3" | "ext4" => Some((16, "bytes")),
        "btrfs" => Some((255, "bytes")),
        "xfs" => Some((12, "bytes")),
        "f2fs" => Some((512, "chars")),
        "nilfs2" => Some((80, "bytes")),
        _ => None,
    }
}
pub(super) fn check_label(lang: &str, fs: &str, label: &Value) -> Result<String, ApiError> {
    let label = label
        .as_str()
        .ok_or_else(|| ApiError::new("bad_args", &tr(lang, "bad_args", &[])))?
        .trim();
    let name = data()["FS_NAMES"][fs].as_str().unwrap_or(fs);
    if label.chars().any(|c| (c as u32) < 32) {
        return Err(ApiError::new(
            "bad_label",
            &tr(lang, "label_bad", &[("fs", name.into())]),
        ));
    }
    let (limit, rule) = label_rule(fs).unwrap_or((64, "chars"));
    let label = if rule == "fat" {
        label.to_uppercase()
    } else {
        label.into()
    };
    if rule == "fat"
        && label
            .chars()
            .any(|c| !c.is_ascii() || "\"*+,./:;<=>?[\\]|".contains(c))
    {
        return Err(ApiError::new(
            "bad_label",
            &tr(lang, "label_bad", &[("fs", name.into())]),
        ));
    }
    let size = if rule == "bytes" {
        label.len()
    } else {
        label.chars().count()
    };
    if size > limit {
        return Err(ApiError::new(
            "bad_label",
            &tr(
                lang,
                "label_long",
                &[("fs", name.into()), ("n", limit.to_string())],
            ),
        ));
    }
    Ok(label)
}
pub(super) fn tools() -> BTreeSet<String> {
    let extra = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(["/usr/sbin", "/sbin", "/usr/local/sbin"].map(PathBuf::from))
        .collect::<Vec<_>>();
    let mut out = BTreeSet::new();
    for fs in arr(&data()["FS_ORDER"]) {
        let spec = &data()["FILESYSTEMS"][string(fs)];
        for prog in arr(&spec[1]) {
            let name = string(prog);
            if extra.iter().any(|p| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(p.join(&name))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }) {
                out.insert(name);
            }
        }
    }
    out
}
pub(super) fn filesystems(
    lang: &str,
    supported: Option<&Vec<Value>>,
    which: &BTreeSet<String>,
) -> Value {
    json!(arr(&data()["FS_ORDER"]).iter().map(|id|{let fs=string(id);let spec=&data()["FILESYSTEMS"][&fs];let known=supported.is_none_or(|a|a.contains(id));let have=arr(&spec[1]).iter().any(|p|which.contains(&string(p)));json!({"id":id,"name":spec[0],"available":known&&have,"maxLabel":spec[2],"labelRule":spec[3],"linux":spec[4],"description":tr(lang,&format!("fs_{fs}"),&[])})}).collect::<Vec<_>>())
}
