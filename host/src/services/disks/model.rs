use super::{labels::*, *};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub(super) struct Block {
    pub path: String,
    pub ifs: Value,
    pub device: String,
    pub devnum: u64,
    pub size: i64,
    pub id_type: String,
    pub usage: String,
    pub label: String,
    pub uuid: String,
    pub version: String,
    pub read_only: bool,
    pub ignore: bool,
    pub hint_name: String,
    pub drive: Option<String>,
    pub backing: Option<String>,
    pub mounts: Vec<String>,
    pub is_fs: bool,
    pub part: Option<Value>,
    pub table: Option<Value>,
    pub crypt: Option<Value>,
    pub swap: bool,
    pub loop_file: Option<String>,
    pub config: Vec<(String, Value)>,
    pub hide: bool,
    pub protected: Vec<String>,
    pub boot_names: Vec<String>,
}
impl Block {
    fn new(path: &str, ifs: &Value) -> Self {
        let b = &ifs[I_BLOCK];
        let device = bstr(&b["Device"]);
        let device = if device.is_empty() {
            bstr(&b["PreferredDevice"])
        } else {
            device
        };
        let mut config = vec![];
        for row in arr(&b["Configuration"]) {
            if row.as_array().is_none_or(|a| a.len() != 2) {
                continue;
            }
            let mut data = row[1].clone();
            if let Some(map) = data.as_object_mut() {
                for (_, v) in map {
                    if v.is_array() {
                        *v = json!(bstr(v))
                    }
                }
            }
            config.push((string(&row[0]), data));
        }
        let loop_file = ifs
            .get(I_LOOP)
            .filter(|p| truth(p))
            .map(|p| bstr(&p["BackingFile"]));
        Self {
            path: path.into(),
            ifs: ifs.clone(),
            device,
            devnum: pyint(&b["DeviceNumber"]).unwrap_or(0) as u64,
            size: int(&b["Size"]),
            id_type: string(&b["IdType"]),
            usage: string(&b["IdUsage"]),
            label: string(&b["IdLabel"]),
            uuid: string(&b["IdUUID"]),
            version: string(&b["IdVersion"]),
            read_only: truth(&b["ReadOnly"]),
            ignore: truth(&b["HintIgnore"]),
            hint_name: string(&b["HintName"]),
            drive: obj(&b["Drive"]),
            backing: obj(&b["CryptoBackingDevice"]),
            mounts: blist(&ifs[I_FS]["MountPoints"]),
            is_fs: ifs.get(I_FS).is_some(),
            part: ifs.get(I_PART).cloned(),
            table: ifs.get(I_TABLE).cloned(),
            crypt: ifs.get(I_CRYPT).cloned(),
            swap: truth(&ifs[I_SWAP]["Active"]),
            loop_file,
            config,
            hide: arr(&b["UserspaceMountOptions"])
                .iter()
                .any(|v| v == "x-gdu.hide" || v == "x-udisks2.hide"),
            protected: vec![],
            boot_names: vec![],
        }
    }
}
pub(super) fn bstr(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.trim_end_matches('\0').into();
    }
    let Some(a) = v.as_array() else {
        return String::new();
    };
    let mut bytes = vec![];
    for v in a {
        let Some(i) = pyint(v).filter(|i| (0..=255).contains(i)) else {
            return String::new();
        };
        bytes.push(i as u8)
    }
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
fn blist(v: &Value) -> Vec<String> {
    arr(v).iter().map(bstr).filter(|s| !s.is_empty()).collect()
}
fn obj(v: &Value) -> Option<String> {
    let s = if truth(v) { string(v) } else { String::new() };
    (!s.is_empty() && s != "/").then_some(s)
}
pub(super) fn kelvin(v: &Value) -> Option<f64> {
    number(v)
        .filter(|n| *n > 200.)
        .map(|n| round(n - 273.15, 1))
}
pub(super) fn hard(reasons: &Value) -> Vec<String> {
    arr(reasons)
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| *s != "boot_entry")
        .map(str::to_owned)
        .collect()
}
pub(super) struct ParseOptions {
    pub language: String,
    pub shell_paths: Vec<String>,
    pub swap_files: Vec<String>,
    pub boot_entries: Vec<Value>,
    pub slaves: BTreeMap<u64, Vec<String>>,
    pub usage: BTreeMap<String, (i64, i64, i64)>,
    pub available_tools: BTreeSet<String>,
}
fn push_unique(list: &mut Vec<String>, s: String) -> bool {
    if list.contains(&s) {
        false
    } else {
        list.push(s);
        true
    }
}
fn protection(blocks: &mut [Block], options: &ParseOptions) {
    let protected = BTreeMap::from([
        ("/", "root"),
        ("/boot", "boot"),
        ("/boot/efi", "efi"),
        ("/efi", "efi"),
        ("/home", "home"),
        ("/usr", "usr"),
        ("/var", "var"),
    ]);
    let mut boot: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &options.boot_entries {
        for uuid in arr(&e["partuuids"]) {
            let names = boot.entry(string(uuid).to_lowercase()).or_default();
            let name = string(&e["name"]);
            if !name.is_empty() {
                push_unique(names, name);
            }
        }
    }
    for b in blocks.iter_mut() {
        for mount in &b.mounts {
            let mm = mount.trim_end_matches('/');
            let mm = if mm.is_empty() { "/" } else { mm };
            if let Some(reason) = protected.get(mm) {
                push_unique(&mut b.protected, reason.to_string());
            }
        }
        for (kind, c) in &b.config {
            let dir = string(&c["dir"]);
            if kind == "fstab" && dir.starts_with('/') {
                let mm = dir.trim_end_matches('/');
                let mm = if mm.is_empty() { "/" } else { mm };
                if let Some(reason) = protected.get(mm) {
                    push_unique(&mut b.protected, reason.to_string());
                }
            }
        }
        if b.swap {
            push_unique(&mut b.protected, "swap".into());
        }
        let uuid = b
            .part
            .as_ref()
            .map(|p| string(&p["UUID"]).to_lowercase())
            .unwrap_or_default();
        if let Some(names) = boot.get(&uuid).filter(|_| !uuid.is_empty()) {
            push_unique(&mut b.protected, "boot_entry".into());
            for name in names {
                push_unique(&mut b.boot_names, name.clone());
            }
        }
    }
    let longest = |blocks: &[Block], path: &str| {
        blocks
            .iter()
            .enumerate()
            .flat_map(|(i, b)| b.mounts.iter().map(move |m| (i, m.trim_end_matches('/'))))
            .filter_map(|(i, m)| {
                let m = if m.is_empty() { "/" } else { m };
                (path == m || path.starts_with(&(m.to_owned() + "/")) || m == "/")
                    .then_some((i, m.len()))
            })
            .max_by_key(|(i, len)| (*len, std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
    };
    for path in &options.shell_paths {
        if !path.is_empty() {
            if let Some(i) = longest(blocks, path) {
                push_unique(&mut blocks[i].protected, "shell".into());
            }
        }
    }
    for path in &options.swap_files {
        if !path.is_empty() {
            if let Some(i) = longest(blocks, path) {
                push_unique(&mut blocks[i].protected, "swap".into());
            }
        }
    }
    let by_path = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.path.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let by_dev = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.device.is_empty())
        .map(|(i, b)| (b.device.clone(), i))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..8 {
        let mut changed = false;
        for i in 0..blocks.len() {
            let b = &blocks[i];
            if b.protected.is_empty() {
                continue;
            }
            let reasons = b.protected.clone();
            let mut under = vec![];
            if let Some(path) = &b.backing {
                if let Some(i) = by_path.get(path) {
                    under.push(*i)
                }
            }
            if b.devnum != 0 && b.part.is_none() && (b.drive.is_none() || b.backing.is_some()) {
                if let Some(slaves) = options.slaves.get(&b.devnum) {
                    for name in slaves {
                        if let Some(i) = by_dev.get(&format!("/dev/{name}")) {
                            under.push(*i)
                        }
                    }
                }
            }
            for u in under {
                for r in &reasons {
                    changed = push_unique(&mut blocks[u].protected, r.clone()) || changed;
                }
            }
        }
        if !changed {
            break;
        }
    }
}
fn drive_kind(drive: &Value, nvme: bool, device: &str) -> &'static str {
    let media = string(&drive["Media"]);
    let compat = arr(&drive["MediaCompatibility"]);
    let bus = string(&drive["ConnectionBus"]);
    let base = device.rsplit('/').next().unwrap_or(device);
    if truth(&drive["Optical"])
        || media.starts_with("optical")
        || compat.iter().any(|m| string(m).starts_with("optical"))
    {
        "optical"
    } else if nvme || base.starts_with("nvme") {
        "nvme"
    } else if bus == "sdio"
        || base.starts_with("mmcblk")
        || media.starts_with("flash_sd")
        || (truth(&drive["MediaRemovable"])
            && compat.iter().any(|m| string(m).starts_with("flash_sd")))
    {
        "sd"
    } else if bus == "usb" {
        "usb"
    } else if pyint(&drive["RotationRate"]) == Some(0) {
        "ssd"
    } else {
        "hdd"
    }
}
pub(super) fn smart_summary(ifs: &Value) -> Value {
    if let Some(nv) = ifs.get(I_NVME) {
        let updated = int(&nv["SmartUpdated"]);
        let warnings = arr(&nv["SmartCriticalWarning"]);
        let status = if updated == 0 {
            "unknown"
        } else if warnings.iter().any(|v| {
            [
                "spare",
                "degraded",
                "readonly",
                "volatile_mem",
                "pmr_readonly",
            ]
            .contains(&string(v).as_str())
        }) {
            "critical"
        } else if !warnings.is_empty() {
            "warning"
        } else {
            "ok"
        };
        return json!({"supported":true,"kind":"nvme","enabled":true,"updated":updated,"healthy":if status=="unknown"{None}else{Some(status=="ok")},"status":status,"temperature":kelvin(&nv["SmartTemperature"]),"powerOnHours":nonzero(&nv["SmartPowerOnHours"]),"criticalWarnings":warnings,"failingAttributes":0,"badSectors":null,"percentUsed":null,"selftest":nonempty(&nv["SmartSelftestStatus"])});
    }
    if let Some(a) = ifs.get(I_ATA).filter(|a| truth(&a["SmartSupported"])) {
        let updated = int(&a["SmartUpdated"]);
        let enabled = truth(&a["SmartEnabled"]);
        let failing = truth(&a["SmartFailing"]);
        let nfail = pyint(&a["SmartNumAttributesFailing"]).unwrap_or(-1);
        let past = pyint(&a["SmartNumAttributesFailedInThePast"]).unwrap_or(-1);
        let bad = pyint(&a["SmartNumBadSectors"]).unwrap_or(-1);
        let hours = int(&a["SmartPowerOnSeconds"]) / 3600;
        let status = if updated == 0 || !enabled {
            "unknown"
        } else if failing || nfail > 0 {
            "critical"
        } else if bad > 0 || past > 0 {
            "warning"
        } else {
            "ok"
        };
        return json!({"supported":true,"kind":"ata","enabled":enabled,"updated":updated,"healthy":if status=="unknown"{None}else{Some(status=="ok")},"status":status,"temperature":kelvin(&a["SmartTemperature"]),"powerOnHours":if hours!=0{Some(hours)}else{None},"criticalWarnings":[],"failingAttributes":nfail.max(0),"failedInPast":past.max(0),"badSectors":if bad>=0{Some(bad)}else{None},"percentUsed":null,"selftest":nonempty(&a["SmartSelftestStatus"])});
    }
    json!({"supported":false,"status":"unknown","healthy":null})
}
fn part_type(part: Option<&Value>, table: Option<&str>) -> (String, String, String) {
    let Some(part) = part.filter(|p| truth(p)) else {
        return (String::new(), "data".into(), String::new());
    };
    let raw = string(&part["Type"]).to_lowercase();
    let kinds = if table == Some("dos") || raw.starts_with("0x") {
        &data()["DOS_TYPES"]
    } else {
        &data()["GPT_TYPES"]
    };
    let kind = if kinds[&raw].is_array() {
        kinds[&raw][0].as_str().unwrap_or("other")
    } else {
        "other"
    };
    let role = if kinds[&raw].is_array() {
        kinds[&raw][1].as_str().unwrap_or("data")
    } else {
        "data"
    };
    if (table == Some("dos") || raw.starts_with("0x")) && truth(&part["IsContainer"]) {
        ("extended".into(), "container".into(), raw)
    } else {
        (kind.into(), role.into(), raw)
    }
}
fn volume_name(
    b: &Block,
    clear: Option<&Block>,
    ptype: &str,
    disk_kind: &str,
    lang: &str,
) -> String {
    let label = clear
        .map(|c| c.label.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| (!b.label.is_empty()).then(|| b.label.clone()))
        .unwrap_or_else(|| b.hint_name.clone());
    if !label.is_empty() {
        return label;
    }
    let mounts = clear
        .map(|c| c.mounts.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| b.mounts.clone());
    let mut reasons = clear.map(|c| c.protected.clone()).unwrap_or_default();
    reasons.extend(b.protected.clone());
    let has = |p: &str| mounts.iter().any(|m| m == p);
    let t = |key| tr(lang, key, &[]);
    if has("/") || reasons.contains(&"root".into()) {
        return t("v_root");
    }
    if has("/home") {
        return t("v_home");
    }
    if has("/boot/efi") || has("/efi") || ptype == "efi" {
        return t("v_efi");
    }
    if has("/boot") || ptype == "xbootldr" {
        return t("v_boot");
    }
    let fs = clear
        .map(|c| c.id_type.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&b.id_type);
    if fs == "swap" || ptype == "swap" {
        return t("v_swap");
    }
    let special = match ptype {
        "msr" => Some("v_msr"),
        "recovery" => Some("v_recovery"),
        "ldm_meta" => Some("v_ldm_meta"),
        "ldm_data" => Some("v_ldm_data"),
        "bios" => Some("v_bios"),
        "extended" => Some("v_extended"),
        "lvm" => Some("v_lvm"),
        "raid" => Some("v_raid"),
        _ => None,
    };
    if let Some(key) = special {
        return t(key);
    }
    if b.crypt.is_some() && clear.is_none() {
        return t("v_locked");
    }
    if fs == "LVM2_member" {
        return t("v_lvm");
    }
    if ["iso9660", "udf"].contains(&fs) && disk_kind == "optical" {
        return t("v_disc");
    }
    if fs.is_empty() {
        return t("v_raw");
    }
    t(if ["usb", "sd"].contains(&disk_kind) {
        "v_usb"
    } else {
        "v_local"
    })
}
fn volume(
    b: &Block,
    blocks: &[Block],
    table: Option<&str>,
    kind: &str,
    opts: &ParseOptions,
    jobs: &BTreeMap<String, Value>,
) -> Value {
    let clear = blocks
        .iter()
        .rev()
        .find(|c| c.backing.as_deref() == Some(&b.path));
    let (ptype, role, raw) = part_type(b.part.as_ref(), table);
    let fsb = clear.unwrap_or(b);
    let mounts = &fsb.mounts;
    let usage = mounts.first().and_then(|p| opts.usage.get(p)).copied();
    let mut protected = b.protected.clone();
    if let Some(clear) = clear {
        for p in &clear.protected {
            push_unique(&mut protected, p.clone());
        }
    }
    let mut fs = if ["filesystem", "other", "raid", "crypto"].contains(&fsb.usage.as_str())
        || !fsb.id_type.is_empty()
    {
        fsb.id_type.clone()
    } else {
        String::new()
    };
    let encrypted = if b.crypt.is_some() || b.id_type == "crypto_LUKS" {
        if clear.is_none() {
            fs = "crypto_LUKS".into()
        }
        json!({"type":format!("{}{}",if b.id_type.is_empty(){"crypto"}else{&b.id_type},if b.version.is_empty(){String::new()}else{format!(" {}",b.version)}),"unlocked":clear.is_some(),"cleartext":clear.map(|c|&c.path),"cleartextDevice":clear.map(|c|&c.device),"inner":clear.map(|c|&c.id_type)})
    } else {
        Value::Null
    };
    let swap = fs == "swap" || ptype == "swap";
    let configs = b
        .config
        .iter()
        .chain(clear.into_iter().flat_map(|c| c.config.iter()))
        .filter(|(k, _)| k == "fstab")
        .map(|(_, c)| c)
        .collect::<Vec<_>>();
    let fstab_label = configs.iter().any(|c| {
        string(&c["fsname"]).starts_with("LABEL=")
            || string(&c["fsname"]).starts_with("/dev/disk/by-label/")
    });
    let empty = json!({});
    let part = b.part.as_ref().unwrap_or(&empty);
    let job = jobs
        .get(&b.path)
        .or_else(|| clear.and_then(|c| jobs.get(&c.path)));
    let is_system = protected.iter().any(|p| p != "boot_entry");
    let mut boot = b.boot_names.clone();
    if let Some(c) = clear {
        for n in &c.boot_names {
            push_unique(&mut boot, n.clone());
        }
    }
    let partition = b.part.as_ref().is_some_and(truth);
    let size = if partition {
        let psize = int(&part["Size"]);
        if psize != 0 { psize } else { b.size }
    } else {
        b.size
    };
    let label_remount = !mounts.is_empty() && OFFLINE_LABEL.contains(&fs.as_str());
    let fs_name = if fs.is_empty() {
        None
    } else {
        Some(
            data()["FS_NAMES"][&fs]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| fs.to_uppercase()),
        )
    };
    json!({"id":b.path,"device":b.device,"label":clear.map(|c|c.label.as_str()).unwrap_or(&b.label),"name":volume_name(b,clear,&ptype,kind,&opts.language),"fsType":if fs.is_empty(){None}else{Some(&fs)},"fsName":fs_name,"fsVersion":if fs.is_empty(){None}else{(!fsb.version.is_empty()).then_some(&fsb.version)},"uuid":(!fsb.uuid.is_empty()).then_some(&fsb.uuid),"size":size,"offset":int(&part["Offset"]),"kind":if partition{"partition"}else{"whole"},"partNumber":nonzero(&part["Number"]),"partType":(!raw.is_empty()).then_some(&raw),"partTypeKey":(!ptype.is_empty()).then_some(&ptype),"partTypeName":if ptype.is_empty(){None}else{Some(tr(&opts.language,&format!("t_{ptype}"),&[("t",raw.clone())]))},"partName":nonempty(&part["Name"]),"role":role,"isContainer":truth(&part["IsContainer"]),"isContained":truth(&part["IsContained"]),"mountPoints":mounts,"mounted":!mounts.is_empty(),"mountPoint":mounts.first(),"used":usage.map(|u|u.1),"free":usage.map(|u|u.2),"total":usage.map(|u|u.0),"isSystem":is_system,"protected":protected,"bootEntries":boot,"isSwap":swap,"swapActive":b.swap||clear.is_some_and(|c|c.swap),"encrypted":encrypted,"readOnly":b.read_only,"hidden":b.ignore,"fstab":!configs.is_empty(),"fstabByLabel":fstab_label,"canMount":fsb.is_fs&&mounts.is_empty(),"canUnmount":fsb.is_fs&&!mounts.is_empty()&&!is_system,"canLabel":fsb.is_fs&&label_rule(&fs).is_some()&&protected.is_empty()&&!b.read_only&&!fstab_label,"labelRemount":label_remount,"canFormat":protected.is_empty()&&!b.read_only&&!matches!(role.as_str(),"system"|"container"),"canDelete":partition&&protected.is_empty()&&!b.read_only&&role!="system"&&ptype!="ldm_data","busy":job.map(|j|json!({"operation":j["operation"],"progress":j["progress"]}))})
}
pub(super) fn parse(objects: &Value, options: &ParseOptions) -> Value {
    let mut blocks = vec![];
    let mut drives = BTreeMap::new();
    let mut jobs = vec![];
    let mut manager = json!({});
    for (path, ifs) in objects.as_object().into_iter().flatten() {
        if ifs.get(I_BLOCK).is_some() {
            blocks.push(Block::new(path, ifs));
        }
        if ifs.get(I_DRIVE).is_some() {
            drives.insert(path.clone(), ifs.clone());
        }
        if let Some(j) = ifs.get(I_JOB) {
            jobs.push(json!({"id":path,"operation":string(&j["Operation"]),"progress":if truth(&j["ProgressValid"]){Some(number(&j["Progress"]).unwrap_or(0.))}else{None},"objects":arr(&j["Objects"]).iter().map(string).collect::<Vec<_>>(),"cancelable":truth(&j["Cancelable"]),"rate":nonzero(&j["Rate"]),"expectedEnd":nonzero(&j["ExpectedEndTime"])}));
        }
        if let Some(m) = ifs.get(I_MANAGER) {
            manager = m.clone();
        }
    }
    jobs.sort_by(|a, b| string(&a["id"]).cmp(&string(&b["id"])));
    let mut jobs_by = BTreeMap::new();
    for j in &jobs {
        for o in arr(&j["objects"]) {
            jobs_by.entry(string(o)).or_insert(j.clone());
        }
    }
    protection(&mut blocks, options);
    let mut wholes = vec![];
    for (i, b) in blocks.iter().enumerate() {
        if b.part.is_some() || b.backing.is_some() {
            continue;
        }
        if let Some(drive) = b.drive.as_ref().and_then(|d| drives.get(d)) {
            wholes.push((
                nonempty(&drive[I_DRIVE]["SortKey"]).unwrap_or_else(|| b.drive.clone().unwrap()),
                i,
                Some(drive.clone()),
            ));
        } else if b.loop_file.is_some() || b.ifs.get(I_LOOP).is_some() {
            if b.ignore
                || b.hide
                || b.size == 0
                || b.loop_file
                    .as_ref()
                    .is_some_and(|s| s.starts_with("/var/lib/snapd/"))
                || b.mounts.iter().any(|m| m.starts_with("/snap/"))
            {
                continue;
            }
            wholes.push((format!("zz-loop/{}", b.device), i, None));
        }
    }
    wholes.sort_by(|a, b| a.0.cmp(&b.0));
    let mut disks = vec![];
    for (n, (_, i, drive_ifs)) in wholes.iter().enumerate() {
        let w = &blocks[*i];
        let empty = json!({});
        let drive = drive_ifs.as_ref().map(|d| &d[I_DRIVE]).unwrap_or(&empty);
        let kind = if drive_ifs.is_none() {
            "loop"
        } else {
            drive_kind(
                drive,
                drive_ifs.as_ref().is_some_and(|d| d.get(I_NVME).is_some()),
                &w.device,
            )
        };
        let table = w.table.as_ref().and_then(|t| nonempty(&t["Type"]));
        let media = drive_ifs.is_none() || drive.get("MediaAvailable").is_none_or(truth);
        let mut volumes = vec![];
        let mut free = vec![];
        let mut primaries = 0;
        if w.table.is_some() {
            let mut parts = blocks
                .iter()
                .filter(|p| p.part.as_ref().is_some_and(|p| p["Table"] == w.path))
                .collect::<Vec<_>>();
            parts.sort_by_key(|p| int(&p.part.as_ref().unwrap()["Offset"]));
            for p in &parts {
                volumes.push(volume(
                    p,
                    &blocks,
                    table.as_deref(),
                    kind,
                    options,
                    &jobs_by,
                ));
            }
            let span = |p: &&Block| {
                let part = p.part.as_ref().unwrap();
                (
                    int(&part["Offset"]),
                    nonzero(&part["Size"]).unwrap_or(p.size),
                )
            };
            let top = parts
                .iter()
                .filter(|p| !truth(&p.part.as_ref().unwrap()["IsContained"]))
                .map(span)
                .collect::<Vec<_>>();
            primaries = top.len();
            let full = table.as_deref() == Some("dos") && primaries >= 4;
            for mut r in free_regions(w.size, table.as_deref(), &top, None) {
                r["canCreate"] = json!(!w.read_only && !full);
                r["container"] = Value::Null;
                free.push(r);
            }
            for c in parts
                .iter()
                .filter(|p| truth(&p.part.as_ref().unwrap()["IsContainer"]))
            {
                let container = span(c);
                let inside = parts
                    .iter()
                    .filter(|p| {
                        let (off, _) = span(p);
                        truth(&p.part.as_ref().unwrap()["IsContained"])
                            && container.0 <= off
                            && off < container.0 + container.1
                    })
                    .map(span)
                    .collect::<Vec<_>>();
                for mut r in free_regions(w.size, table.as_deref(), &inside, Some(container)) {
                    r["canCreate"] = json!(!w.read_only);
                    r["container"] = json!(c.path);
                    free.push(r);
                }
            }
            free.sort_by_key(|f| int(&f["offset"]));
            if volumes.iter().any(|v| truth(&v["isContained"])) {
                for v in &mut volumes {
                    if truth(&v["isContainer"]) {
                        v["canDelete"] = json!(false)
                    }
                }
            }
        } else if w.size > 0 && (!w.usage.is_empty() || !w.id_type.is_empty() || w.crypt.is_some())
        {
            volumes.push(volume(w, &blocks, None, kind, options, &jobs_by));
        } else if w.size > 0 && media {
            free.push(json!({"offset":0,"size":w.size,"logical":false,"canCreate":false,"container":null}));
        }
        let mut reasons = vec![];
        let mut boot = vec![];
        for v in &volumes {
            for r in arr(&v["protected"]) {
                push_unique(&mut reasons, string(r));
            }
            for name in arr(&v["bootEntries"]) {
                push_unique(&mut boot, string(name));
            }
        }
        for r in &w.protected {
            push_unique(&mut reasons, r.clone());
        }
        let system = reasons.iter().any(|r| r != "boot_entry");
        let vendor = string(&drive["Vendor"]).trim().to_owned();
        let model = string(&drive["Model"]).trim().to_owned();
        let title = format!("{vendor} {model}").trim().to_owned();
        let title = if title.is_empty() {
            w.loop_file
                .as_ref()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| w.device.clone())
        } else {
            title
        };
        let job = jobs_by
            .get(&w.path)
            .or_else(|| w.drive.as_ref().and_then(|d| jobs_by.get(d)))
            .cloned()
            .or_else(|| {
                volumes
                    .iter()
                    .find(|v| truth(&v["busy"]))
                    .map(|v| v["busy"].clone())
            });
        disks.push(json!({"id":w.path,"drive":if drive_ifs.is_some(){w.drive.clone()}else{None},"block":w.path,"device":w.device,"number":n,"name":tr(&options.language,"disk",&[("n",n.to_string())]),"model":title,"vendor":(!vendor.is_empty()).then_some(vendor),"serial":nonempty(&drive["Serial"]),"revision":nonempty(&drive["Revision"]),"wwn":nonempty(&drive["WWN"]),"size":if w.size!=0{w.size}else{int(&drive["Size"])},"kind":kind,"bus":nonempty(&drive["ConnectionBus"]).or_else(||(kind=="nvme").then(||"nvme".into())),"rotationRate":if drive_ifs.is_some(){Some(int(&drive["RotationRate"]))}else{None},"removable":truth(&drive["Removable"])||["usb","sd","optical"].contains(&kind),"mediaRemovable":truth(&drive["MediaRemovable"]),"media":media,"ejectable":truth(&drive["Ejectable"]),"canPowerOff":truth(&drive["CanPowerOff"]),"readOnly":w.read_only,"loopFile":w.loop_file,"isSystem":system,"protected":reasons,"bootEntries":boot,"canEject":!system,"canInitialize":reasons.is_empty()&&!w.read_only&&kind!="optical","smart":smart_summary(drive_ifs.as_ref().unwrap_or(&empty)),"partitionTable":table,"initialized":w.table.is_some()||!volumes.is_empty(),"canPartition":w.table.is_some()&&!w.read_only&&free.iter().any(|f|truth(&f["canCreate"])),"tableFull":table.as_deref()==Some("dos")&&primaries>=4,"dynamic":volumes.iter().any(|v|v["partTypeKey"]=="ldm_meta"||v["partTypeKey"]=="ldm_data"),"volumes":volumes,"free":free,"busy":job.map(|j|json!({"operation":j["operation"],"progress":j["progress"]}))}));
    }
    let supported = manager["SupportedFilesystems"]
        .as_array()
        .filter(|a| !a.is_empty());
    json!({"supported":true,"disks":disks,"jobs":jobs,"filesystems":filesystems(&options.language,supported,&options.available_tools),"encryption":{"supported":truth(&manager["SupportedEncryptionTypes"]),"types":arr(&manager["SupportedEncryptionTypes"]).iter().map(string).collect::<Vec<_>>(),"default":nonempty(&manager["DefaultEncryptionType"]).unwrap_or_else(||"luks2".into())},"version":nonempty(&manager["Version"])})
}
pub(super) fn free_regions(
    size: i64,
    table: Option<&str>,
    parts: &[(i64, i64)],
    container: Option<(i64, i64)>,
) -> Vec<Value> {
    let (first, last, lead, tail) = if let Some((off, size)) = container {
        (off, off + size, MIB, MIB)
    } else {
        if size <= 0 {
            return vec![];
        }
        (
            if table == Some("gpt") { 34 * 512 } else { 512 },
            if table == Some("gpt") {
                size - 33 * 512
            } else {
                size
            },
            0,
            0,
        )
    };
    let mut out = vec![];
    let mut gap = |a: i64, b: i64| {
        let a = a.div_euclid(MIB) * MIB + if a.rem_euclid(MIB) != 0 { MIB } else { 0 } + lead;
        let b = b.div_euclid(MIB) * MIB - tail;
        if b - a >= MIN_VOLUME {
            out.push(json!({"offset":a,"size":b-a,"logical":container.is_some()}));
        }
    };
    let mut parts = parts.to_vec();
    parts.sort();
    let mut cur = first;
    for (off, size) in parts {
        if off - cur >= MIN_VOLUME {
            gap(cur, off)
        }
        cur = cur.max(off + size);
    }
    gap(cur, last);
    out
}
pub(super) fn nvme_health(a: &Value) -> Value {
    let n = |k: &str| a[k].is_number().then(|| pyint(&a[k])).flatten();
    let used = n("percent_used");
    let mut out = json!({"percentUsed":used,"lifeRemaining":used.map(|u|(100-u).max(0)),"availableSpare":n("avail_spare"),"spareThreshold":n("spare_thresh"),"dataRead":n("total_data_read"),"dataWritten":n("total_data_written"),"powerCycles":n("power_cycles"),"unsafeShutdowns":n("unsafe_shutdowns"),"mediaErrors":n("media_errors"),"errorLogEntries":n("num_err_log_entries"),"warningTempMinutes":n("warning_temp_time"),"criticalTempMinutes":n("critical_temp_time"),"details":true});
    if let Some(temps) = a["temp_sensors"].as_array() {
        out["sensors"] = json!(temps.iter().filter_map(kelvin).collect::<Vec<_>>())
    }
    out
}
pub(super) fn ata_attributes(rows: &Value) -> Value {
    json!(arr(rows).iter().filter(|r|r.as_array().is_some_and(|a|a.len()==9)).map(|r|{let value=int(&r[3]);let worst=int(&r[4]);let thresh=int(&r[5]);json!({"id":int(&r[0]),"name":string(&r[1]),"prefail":int(&r[2])&1!=0,"value":(value>=0).then_some(value),"worst":(worst>=0).then_some(worst),"threshold":(thresh>=0).then_some(thresh),"pretty":int(&r[6]),"unit":int(&r[7]),"failing":value>=0&&thresh>0&&value<=thresh,"failedInPast":worst>=0&&thresh>0&&worst<=thresh})}).collect::<Vec<_>>())
}
