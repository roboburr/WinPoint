use super::*;
use labels::{check_label, label_rule};
pub(super) fn jobs_on(snap: &Value, paths: &BTreeSet<String>) -> Vec<Value> {
    arr(&snap["jobs"])
        .iter()
        .filter(|j| {
            arr(&j["objects"])
                .iter()
                .any(|o| paths.contains(&string(o)))
        })
        .cloned()
        .collect()
}
pub(super) fn volume_paths(v: &Value) -> BTreeSet<String> {
    [nonempty(&v["id"]), nonempty(&v["encrypted"]["cleartext"])]
        .into_iter()
        .flatten()
        .collect()
}
pub(super) fn disk_paths(disks: &[Value]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for d in disks {
        out.extend(
            [nonempty(&d["block"]), nonempty(&d["drive"])]
                .into_iter()
                .flatten(),
        );
        for v in arr(&d["volumes"]) {
            out.extend(volume_paths(v));
        }
    }
    out
}
pub(super) fn siblings(snap: &Value, disk: &Value) -> Vec<Value> {
    if !truth(&disk["drive"]) {
        return vec![disk.clone()];
    }
    let all = arr(&snap["disks"])
        .iter()
        .filter(|d| d["drive"] == disk["drive"])
        .cloned()
        .collect::<Vec<_>>();
    if all.is_empty() {
        vec![disk.clone()]
    } else {
        all
    }
}
pub(super) fn keys(snap: &Value, disk: &Value) -> Vec<String> {
    siblings(snap, disk)
        .iter()
        .map(|d| nonempty(&d["drive"]).unwrap_or_else(|| string(&d["block"])))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
impl Disks {
    fn check_id(&self, v: &Value) -> Result<String, ApiError> {
        let valid = |s: &str| {
            let Some(tail) = s.strip_prefix("/org/freedesktop/UDisks2/") else {
                return false;
            };
            let Some((group, id)) = tail.split_once('/') else {
                return false;
            };
            ["block_devices", "drives"].contains(&group)
                && (1..=160).contains(&id.len())
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        v.as_str()
            .filter(|s| valid(s))
            .map(str::to_owned)
            .ok_or_else(|| self.err("bad_args", "bad_args", &[]))
    }
    pub(super) fn find_volume(&self, snap: &Value, id: &Value) -> Result<(Value, Value), ApiError> {
        let id = self.check_id(id)?;
        for d in arr(&snap["disks"]) {
            for v in arr(&d["volumes"]) {
                if v["id"] == id || v["encrypted"]["cleartext"] == id {
                    return Ok((d.clone(), v.clone()));
                }
            }
        }
        Err(self.err("not_found", "gone_volume", &[]))
    }
    pub(super) fn find_disk(&self, snap: &Value, id: &Value) -> Result<Value, ApiError> {
        let id = self.check_id(id)?;
        if let Some(d) = arr(&snap["disks"])
            .iter()
            .find(|d| d["id"] == id || d["block"] == id)
        {
            return Ok(d.clone());
        }
        let same = arr(&snap["disks"])
            .iter()
            .filter(|d| d["drive"] == id)
            .collect::<Vec<_>>();
        if same.len() == 1 {
            return Ok(same[0].clone());
        }
        Err(self.err("not_found", "gone_disk", &[]))
    }
    pub(super) fn refuse(
        &self,
        reasons: &[Value],
        op: &str,
        disk: bool,
        names: &[Value],
    ) -> Result<(), ApiError> {
        if reasons.is_empty() {
            return Ok(());
        }
        let hard = reasons
            .iter()
            .filter(|r| r.as_str() != Some("boot_entry"))
            .collect::<Vec<_>>();
        let names = names
            .iter()
            .map(|n| format!("‘{}’", string(n)))
            .collect::<Vec<_>>()
            .join(", ");
        let names = if names.is_empty() { "?".into() } else { names };
        let key = if disk {
            if hard.is_empty() {
                "r_disk_boot".into()
            } else {
                "r_disk".into()
            }
        } else {
            format!("r_{}", string(hard.first().copied().unwrap_or(&reasons[0])))
        };
        let why = tr(&self.language(), &key, &[("names", names)]);
        let mut error = ApiError::new(
            "protected",
            &format!("{why} {}", tr(&self.language(), &format!("no_{op}"), &[])),
        );
        error.data = Some(json!({"reasons":reasons}));
        Err(error)
    }
    pub(super) fn no_jobs(
        &self,
        snap: &Value,
        paths: &BTreeSet<String>,
        name: &str,
    ) -> Result<(), ApiError> {
        if !jobs_on(snap, paths).is_empty() {
            Err(self.err("busy_job", "busy_job", &[("name", name.into())]))
        } else {
            Ok(())
        }
    }
    pub(super) fn expect(&self, obj: &Value, expect: &Value) -> Result<(), ApiError> {
        if !expect.is_object() || expect.get("size").is_none() || expect.get("device").is_none() {
            return Err(self.err("bad_args", "expect", &[]));
        }
        let same = pyint(&expect["size"])
            .zip(pyint(&obj["size"]))
            .is_some_and(|(a, b)| a == b)
            && string(&expect["device"]) == string(&obj["device"]);
        if !same || (truth(&expect["uuid"]) && truth(&obj["uuid"]) && expect["uuid"] != obj["uuid"])
        {
            return Err(self.err("changed", "changed", &[]));
        }
        Ok(())
    }
    pub(super) fn volume_checks(
        &self,
        snap: &Value,
        op: &str,
        args: &Value,
    ) -> Result<(Value, Value), ApiError> {
        let (d, v) = self.find_volume(snap, &args["id"])?;
        let paths = volume_paths(&v);
        let name = string(&v["name"]);
        match op {
            "unmount" => {
                let hard = model::hard(&v["protected"])
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>();
                self.refuse(&hard, "unmount", false, &[])?;
                if !truth(&v["mounted"]) {
                    return Err(self.err("not_mounted", "not_mounted", &[]));
                }
                self.no_jobs(snap, &paths, &name)?
            }
            "mount" | "open" => {
                if !truth(&v["mounted"]) {
                    self.mount_target(&v)?;
                    self.no_jobs(snap, &paths, &name)?
                }
            }
            "setLabel" => {
                self.refuse(arr(&v["protected"]), "label", false, arr(&v["bootEntries"]))?;
                self.label_target(&v, &args["label"])?;
                self.no_jobs(snap, &paths, &name)?
            }
            "format" => {
                self.refuse(
                    arr(&v["protected"]),
                    "format",
                    false,
                    arr(&v["bootEntries"]),
                )?;
                self.expect(&v, &args["expect"])?;
                self.formattable(&v)?;
                self.format_args(snap, args, v["kind"] == "partition")?;
                self.no_jobs(snap, &paths, &name)?
            }
            "deletePartition" => {
                self.refuse(
                    arr(&v["protected"]),
                    "delete",
                    false,
                    arr(&v["bootEntries"]),
                )?;
                self.expect(&v, &args["expect"])?;
                self.delete_target(&d, &v)?;
                self.no_jobs(snap, &paths, &name)?
            }
            "unlock" => {
                if !truth(&v["encrypted"]) {
                    return Err(self.err("not_encrypted", "not_encrypted", &[]));
                }
                if !args["passphrase"].as_str().is_some_and(|s| !s.is_empty()) {
                    return Err(self.err("bad_args", "passphrase_empty", &[]));
                }
            }
            "lock" => {
                if !truth(&v["encrypted"]) {
                    return Err(self.err("not_encrypted", "not_encrypted", &[]));
                }
                let hard = model::hard(&v["protected"])
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>();
                self.refuse(&hard, "lock", false, &[])?;
                self.no_jobs(snap, &paths, &name)?
            }
            _ => {}
        }
        Ok((d, v))
    }
    pub(super) fn disk_checks(
        &self,
        snap: &Value,
        op: &str,
        args: &Value,
    ) -> Result<Value, ApiError> {
        let d = self.find_disk(
            snap,
            if truth(&args["disk"]) {
                &args["disk"]
            } else {
                &args["id"]
            },
        )?;
        match op {
            "eject" | "powerOff" => {
                let sib = siblings(snap, &d);
                let hard = sib
                    .iter()
                    .flat_map(|d| model::hard(&d["protected"]))
                    .map(Value::String)
                    .collect::<Vec<_>>();
                self.refuse(&hard, "eject", true, &[])?;
                if d["kind"] == "loop" || !truth(&d["drive"]) {
                    return Err(self.err("not_removable", "not_removable", &[]));
                }
                self.no_jobs(snap, &disk_paths(&sib), &string(&d["model"]))?
            }
            "initialize" => {
                self.refuse(
                    arr(&d["protected"]),
                    "initialize",
                    true,
                    arr(&d["bootEntries"]),
                )?;
                self.expect(&d, &args["expect"])?;
                if args
                    .get("table")
                    .is_some_and(|table| !matches!(table.as_str(), Some("gpt" | "dos")))
                {
                    return Err(self.err("bad_args", "bad_table", &[]));
                }
                self.no_jobs(snap, &disk_paths(&[d.clone()]), &string(&d["name"]))?
            }
            "createPartition" => {
                if !args["expect"].is_null() {
                    self.expect(&d, &args["expect"])?;
                }
                self.create_args(snap, &d, args)?;
                self.no_jobs(snap, &disk_paths(&[d.clone()]), &string(&d["name"]))?
            }
            _ => {}
        }
        Ok(d)
    }
    pub(super) fn plan(&self, op: &str, args: &Value) -> ApiResult {
        let snap = self.fresh()?;
        if [
            "mount",
            "unmount",
            "open",
            "setLabel",
            "format",
            "deletePartition",
            "unlock",
            "lock",
        ]
        .contains(&op)
        {
            let (_, v) = self.volume_checks(&snap, op, args)?;
            Ok(json!({"id":v["id"],"device":v["device"]}))
        } else {
            let d = self.disk_checks(&snap, op, args)?;
            Ok(json!({"disk":d["id"],"device":d["device"]}))
        }
    }
    pub(super) fn mount_target(&self, v: &Value) -> Result<String, ApiError> {
        let enc = &v["encrypted"];
        if truth(enc) && !truth(&enc["unlocked"]) {
            return Err(self.err("locked", "locked", &[]));
        }
        if !truth(&v["fsType"])
            || ["swap", "crypto_LUKS", "LVM2_member", "linux_raid_member"]
                .contains(&string(&v["fsType"]).as_str())
            || (truth(enc) && !truth(&enc["inner"]))
        {
            return Err(self.err("no_fs", "no_fs", &[]));
        }
        Ok(string(if truth(enc) {
            &enc["cleartext"]
        } else {
            &v["id"]
        }))
    }
    pub(super) fn label_target(
        &self,
        v: &Value,
        label: &Value,
    ) -> Result<(String, String), ApiError> {
        let enc = &v["encrypted"];
        if truth(enc) && !truth(&enc["unlocked"]) {
            return Err(self.err("locked", "locked", &[]));
        }
        let fs = string(&v["fsType"]);
        if label_rule(&fs).is_none() {
            return Err(self.err("no_label_fs", "no_label_fs", &[]));
        }
        if truth(&v["fstabByLabel"]) {
            return Err(self.err("in_use", "fstab_label", &[]));
        }
        let empty = json!("");
        Ok((
            string(if truth(enc) {
                &enc["cleartext"]
            } else {
                &v["id"]
            }),
            check_label(
                &self.language(),
                &fs,
                if label.is_null() { &empty } else { label },
            )?,
        ))
    }
    fn fs_ok(&self, snap: &Value, fs: &Value) -> Result<String, ApiError> {
        let fs = fs
            .as_str()
            .filter(|s| data()["FILESYSTEMS"].get(s).is_some())
            .ok_or_else(|| self.err("bad_args", "fs_unknown", &[]))?;
        if !arr(&snap["filesystems"])
            .iter()
            .any(|f| f["id"] == fs && truth(&f["available"]))
        {
            return Err(self.err(
                "fs_missing",
                "fs_missing",
                &[("fs", string(&data()["FILESYSTEMS"][fs][0]))],
            ));
        }
        Ok(fs.into())
    }
    fn passphrase(&self, snap: &Value, args: &Value) -> Result<Option<String>, ApiError> {
        let enc = &args["encrypt"];
        if !truth(enc) {
            return Ok(None);
        }
        let passphrase = enc["passphrase"]
            .as_str()
            .filter(|s| s.chars().count() >= 8)
            .ok_or_else(|| self.err("bad_args", "passphrase", &[]))?;
        if !truth(&snap["encryption"]["supported"]) {
            return Err(self.err("not_supported", "failed", &[("detail", "LUKS".into())]));
        }
        Ok(Some(passphrase.into()))
    }
    fn formattable(&self, v: &Value) -> Result<(), ApiError> {
        if matches!(v["role"].as_str(), Some("system" | "container")) {
            return Err(self.err(
                "system_partition",
                "system_part",
                &[(
                    "type",
                    nonempty(&v["partTypeName"]).unwrap_or_else(|| string(&v["name"])),
                )],
            ));
        }
        Ok(())
    }
    pub(super) fn format_args(&self, snap: &Value, args: &Value, partition: bool) -> ApiResult {
        let fs = self.fs_ok(snap, &args["fsType"])?;
        let empty = json!("");
        let label = check_label(
            &self.language(),
            &fs,
            if truth(&args["label"]) {
                &args["label"]
            } else {
                &empty
            },
        )?;
        let pw = self.passphrase(snap, args)?;
        let mut opts = json!({"tear-down":true});
        if !label.is_empty() {
            opts["label"] = json!(label)
        }
        if partition {
            opts["update-partition-type"] = json!(true)
        }
        if truth(&data()["FILESYSTEMS"][&fs][4]) {
            opts["take-ownership"] = json!(true)
        }
        if args["quick"] == false {
            opts["erase"] = json!("zero")
        }
        if let Some(pw) = pw {
            opts["encrypt.passphrase"] = json!(pw);
            opts["encrypt.type"] =
                json!(nonempty(&snap["encryption"]["default"]).unwrap_or_else(|| "luks2".into()))
        }
        Ok(json!({"fs":fs,"opts":opts}))
    }
    fn delete_target(&self, d: &Value, v: &Value) -> Result<(), ApiError> {
        if v["kind"] != "partition" {
            return Err(self.err("not_partition", "not_partition", &[]));
        }
        if v["role"] == "system" {
            return Err(self.err(
                "system_partition",
                "system_part",
                &[(
                    "type",
                    nonempty(&v["partTypeName"]).unwrap_or_else(|| string(&v["name"])),
                )],
            ));
        }
        if v["partTypeKey"] == "ldm_data" {
            return Err(self.err("dynamic_partition", "dynamic_part", &[]));
        }
        if truth(&v["isContainer"]) && arr(&d["volumes"]).iter().any(|v| truth(&v["isContained"])) {
            return Err(self.err("not_empty", "has_logical", &[]));
        }
        Ok(())
    }
    pub(super) fn create_args(&self, snap: &Value, d: &Value, args: &Value) -> ApiResult {
        if !truth(&d["partitionTable"]) {
            return Err(self.err("no_table", "no_table", &[]));
        }
        if truth(&d["readOnly"]) {
            return Err(self.err("read_only", "failed", &[("detail", "read-only".into())]));
        }
        let offset = pyint(&args["offset"]).ok_or_else(|| self.err("bad_args", "bad_args", &[]))?;
        let mut size = if truth(&args["size"]) {
            pyint(&args["size"]).ok_or_else(|| self.err("bad_args", "bad_args", &[]))?
        } else {
            0
        };
        let region = arr(&d["free"])
            .iter()
            .find(|r| int(&r["offset"]) <= offset && offset < int(&r["offset"]) + int(&r["size"]))
            .ok_or_else(|| self.err("no_free", "no_free", &[]))?;
        if region.get("canCreate").is_some_and(|v| !truth(v)) {
            return Err(if truth(&d["tableFull"]) {
                self.err("table_full", "table_full", &[])
            } else {
                self.err("no_free", "no_free", &[])
            });
        }
        let room = int(&region["offset"]) + int(&region["size"]) - offset;
        if size <= 0 || size > room {
            size = room
        }
        if size < MIN_VOLUME {
            return Err(self.err("too_small", "too_small", &[]));
        }
        if size < room && room - size < MIB {
            size = room
        }
        let mut fmt = self.format_args(snap, args, false)?;
        fmt["opts"]
            .as_object_mut()
            .unwrap()
            .remove("update-partition-type");
        let logical = truth(&region["logical"]);
        let fs = string(&fmt["fs"]);
        let ptype = if d["partitionTable"] == "gpt" {
            data()["GPT_FOR_FS"][&fs].clone()
        } else {
            data()["DOS_FOR_FS"][&fs].clone()
        };
        let popts = if d["partitionTable"] == "dos" {
            json!({"partition-type":if logical{"logical"}else{"primary"}})
        } else {
            json!({})
        };
        Ok(
            json!({"offset":offset,"size":size,"type":ptype,"opts":popts,"fs":fs,"fopts":fmt["opts"],"logical":logical}),
        )
    }
}
