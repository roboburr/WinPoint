use super::common::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Mutex, OnceLock},
};
pub(super) fn unescape(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let code = (bytes[i + 1] - b'0') as u32 * 64
                + (bytes[i + 2] - b'0') as u32 * 8
                + (bytes[i + 3] - b'0') as u32;
            if let Some(c) = char::from_u32(code) {
                out.push(c);
                i += 4;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}
pub(super) fn parse_mountinfo(text: &str) -> Vec<Value> {
    text.lines().filter_map(|l|{let p=l.split(' ').collect::<Vec<_>>();let sep=p.iter().position(|s|*s=="-")?;if p.len()<sep+3||sep<6{return None}Some(json!({"mm":p[2],"root":unescape(p[3]),"mount":unescape(p[4]),"opts":p[5],"fstype":p[sep+1],"source":unescape(p[sep+2])}))}).collect()
}
fn lsblk() -> BTreeMap<String, Value> {
    static CACHE: OnceLock<Mutex<(f64, BTreeMap<String, Value>)>> = OnceLock::new();
    let mut state = CACHE
        .get_or_init(|| Mutex::new((0., BTreeMap::new())))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !state.1.is_empty() && now() - state.0 < 10. {
        return state.1.clone();
    }
    let r = run(
        "lsblk",
        &[
            "-J",
            "-b",
            "-o",
            "NAME,LABEL,MODEL,ROTA,RM,TRAN,MOUNTPOINTS,UUID,MAJ:MIN,SIZE,FSTYPE,TYPE,HOTPLUG",
        ],
        4.,
    );
    let data = if r.ok {
        serde_json::from_str::<Value>(&r.out).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut map = BTreeMap::new();
    fn walk(n: &Value, top: &Value, map: &mut BTreeMap<String, Value>) {
        let mm = n["maj:min"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| n["maj_min"].as_str())
            .unwrap_or("");
        map.insert(mm.into(), json!({"node":n,"disk":top}));
        for c in n["children"].as_array().into_iter().flatten() {
            walk(c, top, map)
        }
    }
    for d in data["blockdevices"].as_array().into_iter().flatten() {
        walk(d, d, &mut map)
    }
    *state = (now(), map.clone());
    map
}
fn flag(v: &Value) -> bool {
    v == true || v == 1 || v == "1" || v == "true"
}
pub(super) fn fmt_size(lang: &str, n: u64) -> String {
    let mut v = n as f64;
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    if v < 1024. {
        return format!("{n} bytes");
    }
    v /= 1024.;
    while v >= 1024. && i < 4 {
        v /= 1024.;
        i += 1
    }
    let s = if v >= 10. {
        format!("{:.0}", round(v, 0))
    } else {
        format!("{:.1}", round(v, 1))
    };
    format!(
        "{} {}",
        if lang == "en" { s } else { s.replace('.', ",") },
        units[i]
    )
}
fn vm(name: &str) -> bool {
    [
        "vm",
        "vms",
        "vmstore",
        "vm-store",
        "virtual",
        "virtualmachines",
        "virtual-machines",
        "virtual_machines",
        "virtual machines",
        "libvirt",
        "kvm",
        "qemu",
    ]
    .contains(&name.to_lowercase().as_str())
}
pub(super) fn volumes(lang: &str, include: bool) -> Vec<Value> {
    build(
        lang,
        include,
        parse_mountinfo(&read("/proc/self/mountinfo")),
        &lsblk(),
        &home(),
    )
}
fn build(
    lang: &str,
    include: bool,
    mut mounts: Vec<Value>,
    blk: &BTreeMap<String, Value>,
    home: &Path,
) -> Vec<Value> {
    let home_dev = fs::metadata(home).ok().map(|s| s.dev());
    mounts.sort_by(|a, b| {
        let a = a["mount"].as_str().unwrap_or("");
        let b = b["mount"].as_str().unwrap_or("");
        (a != "/")
            .cmp(&(b != "/"))
            .then(a.len().cmp(&b.len()))
            .then(a.cmp(b))
    });
    let real = data()["_REAL_FS"].as_array().unwrap();
    let skip = data()["_SKIP_MOUNT"].as_array().unwrap();
    let mut seen = HashSet::new();
    let mut out = vec![];
    for m in mounts {
        let (fs, mp) = (
            m["fstype"].as_str().unwrap_or(""),
            m["mount"].as_str().unwrap_or(""),
        );
        if !real.iter().any(|s| s == fs)
            || m["source"]
                .as_str()
                .is_some_and(|s| s.starts_with("/dev/loop"))
            || skip
                .iter()
                .any(|s| s.as_str().is_some_and(|s| mp.starts_with(s)))
            || mp.starts_with("/run/user/")
            || mp.starts_with("/run/") && !mp.starts_with("/run/media/")
        {
            continue;
        }
        let hidden =
            ["/boot", "/boot/efi", "/efi", "/recovery"].contains(&mp) || mp.starts_with("/boot/");
        if hidden && !include {
            continue;
        }
        let mm = m["mm"].as_str().unwrap_or("");
        if !seen.insert(mm.to_owned()) {
            continue;
        }
        let Some(sv) = statvfs(Path::new(mp)) else {
            continue;
        };
        let total = (sv.f_blocks as u64).saturating_mul(sv.f_frsize as u64);
        if total == 0 {
            continue;
        }
        let free = (sv.f_bavail as u64).saturating_mul(sv.f_frsize as u64);
        let used = (sv.f_blocks as u64)
            .saturating_sub(sv.f_bfree as u64)
            .saturating_mul(sv.f_frsize as u64);
        let info = blk.get(mm).unwrap_or(&Value::Null);
        let n = &info["node"];
        let d = &info["disk"];
        let label = n["label"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let uuid = n["uuid"].as_str().filter(|s| !s.is_empty());
        let tran = d["tran"].as_str().unwrap_or("").to_lowercase();
        let disk_name = d["name"].as_str().unwrap_or("");
        let removable = flag(&d["rm"]) || flag(&d["hotplug"]) || tran == "usb";
        let kind = if fs == "iso9660" || fs == "udf" || disk_name.starts_with("sr") {
            "optical"
        } else if tran == "nvme" || disk_name.starts_with("nvme") {
            "nvme"
        } else if tran == "usb" {
            "usb"
        } else if flag(&d["rota"]) {
            "hdd"
        } else {
            "ssd"
        };
        let dev = fs::metadata(mp).ok().map(|s| s.dev());
        let is_system = mp == "/";
        let is_home = home_dev.is_some() && home_dev == dev;
        let base = mp.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let name = if is_system {
            tr(lang, "system", &[])
        } else if label.is_some_and(vm) || label.is_none() && vm(base) {
            tr(lang, "vm", &[])
        } else if let Some(label) = label {
            label.into()
        } else if is_home && Path::new(mp) == home || mp == "/home" {
            tr(lang, "home", &[])
        } else {
            tr(lang, "sized", &[("size", fmt_size(lang, total))])
        };
        let id = uuid
            .map(|s| {
                s.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect::<String>()
            })
            .unwrap_or_else(|| format!("mm-{}", mm.replace(':', "-")));
        let source = m["source"].as_str().unwrap_or("");
        out.push(json!({"id":id,"device":if source.starts_with("/dev/"){source.to_owned()}else{format!("/dev/{}",n["name"].as_str().unwrap_or(""))},"label":label,"name":name,"mount":mp,"fsType":data()["_FS_LABEL"][fs].as_str().unwrap_or(fs),"total":total,"used":used,"free":free,"removable":removable,"isSystem":is_system,"isHome":is_home,"kind":kind,"fsTypeRaw":fs,"percent":round(used as f64*100./used.saturating_add(free).max(1)as f64,1),"readOnly":m["opts"].as_str().unwrap_or("").split(',').any(|s|s=="ro"),"model":d["model"].as_str().map(str::trim).filter(|s|!s.is_empty()),"hidden":hidden}));
    }
    out.sort_by(|a, b| {
        (!truth(&a["isSystem"]))
            .cmp(&!truth(&b["isSystem"]))
            .then((!truth(&a["isHome"])).cmp(&!truth(&b["isHome"])))
            .then(truth(&a["removable"]).cmp(&truth(&b["removable"])))
            .then(truth(&a["hidden"]).cmp(&truth(&b["hidden"])))
            .then(
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase()),
            )
    });
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_mount_fields_and_bindroots_are_retained() {
        let p = parse_mountinfo(
            "26 1 8:2 /a\\040b /mnt/a\\040b rw,relatime - ext4 /dev/name\\134x rw\ninvalid",
        );
        assert_eq!(p[0]["mount"], "/mnt/a b");
        assert_eq!(p[0]["root"], "/a b");
        assert_eq!(p[0]["source"], "/dev/name\\x");
    }
    #[test]
    fn drive_size_labels_match_both_languages() {
        assert_eq!(fmt_size("nl", 1024 * 1024 * 1024), "1,0 GB");
        assert_eq!(fmt_size("en", 22 * 1024 * 1024 * 1024 * 1024), "22 TB");
    }
}
