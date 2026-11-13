use super::*;
use checks::{disk_paths, keys, siblings, volume_paths};
fn opts(value: &Value) -> BTreeMap<String, glib::Variant> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            (
                k.clone(),
                if let Some(b) = v.as_bool() {
                    b.to_variant()
                } else if v.is_number() {
                    (int(v) as u64).to_variant()
                } else {
                    string(v).to_variant()
                },
            )
        })
        .collect()
}
fn empty() -> BTreeMap<String, glib::Variant> {
    BTreeMap::new()
}
impl Disks {
    fn unmount_path(&self, path: &str, name: &str) -> Result<(), ApiError> {
        self.dbus(
            path,
            I_FS,
            "Unmount",
            Some(&(empty(),).to_variant()),
            self.timeout(),
            name,
        )
        .map(|_| ())
    }
    fn teardown(&self, v: &Value) -> Result<bool, ApiError> {
        let mut mounted = false;
        if truth(&v["encrypted"]) && truth(&v["encrypted"]["unlocked"]) {
            if truth(&v["mounted"]) {
                self.unmount_path(
                    v["encrypted"]["cleartext"].as_str().unwrap_or("/"),
                    v["name"].as_str().unwrap_or(""),
                )?;
                mounted = true;
            }
            self.dbus(
                v["id"].as_str().unwrap_or("/"),
                I_CRYPT,
                "Lock",
                Some(&(empty(),).to_variant()),
                self.timeout(),
                v["name"].as_str().unwrap_or(""),
            )?;
        } else if truth(&v["mounted"]) {
            self.unmount_path(
                v["id"].as_str().unwrap_or("/"),
                v["name"].as_str().unwrap_or(""),
            )?;
            mounted = true;
        }
        Ok(mounted)
    }
    fn mount_new(&self, path: &str, name: &str) -> (Option<String>, Option<String>) {
        let result = (|| -> Result<Option<String>, ApiError> {
            let snap = self.snapshot(true)?;
            let (_, v) = self.find_volume(&snap, &json!(path))?;
            let target = self.mount_target(&v)?;
            let r = self.dbus(
                &target,
                I_FS,
                "Mount",
                Some(&(empty(),).to_variant()),
                self.timeout(),
                name,
            )?;
            Ok(nonempty(&first(&r)))
        })();
        match result {
            Ok(path) => (path, None),
            Err(e) => (None, Some(e.message)),
        }
    }
    pub(super) fn mount(&self, id: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (_, v) = self.find_volume(&snap, id)?;
        if truth(&v["mounted"]) {
            return Ok(json!({"ok":true,"mountPoint":v["mountPoint"],"already":true}));
        }
        self.volume_checks(&snap, "mount", &json!({"id":id}))?;
        let target = self.mount_target(&v)?;
        let r = self.dbus(
            &target,
            I_FS,
            "Mount",
            Some(&(empty(),).to_variant()),
            self.timeout(),
            v["name"].as_str().unwrap_or(""),
        )?;
        self.done();
        Ok(json!({"ok":true,"mountPoint":first(&r)}))
    }
    pub(super) fn unmount(&self, id: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (d, v) = self.find_volume(&snap, id)?;
        let hard = model::hard(&v["protected"])
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        self.refuse(&hard, "unmount", false, &[])?;
        if !truth(&v["mounted"]) {
            return Ok(json!({"ok":true,"already":true}));
        }
        {
            let _op = self.operation(keys(&snap, &d))?;
            self.volume_checks(&snap, "unmount", &json!({"id":id}))?;
            let target = if truth(&v["encrypted"]) && truth(&v["encrypted"]["unlocked"]) {
                &v["encrypted"]["cleartext"]
            } else {
                &v["id"]
            };
            self.unmount_path(
                target.as_str().unwrap_or("/"),
                v["name"].as_str().unwrap_or(""),
            )?;
        }
        self.done();
        Ok(json!({"ok":true}))
    }
    pub(super) fn open(&self, id: &Value, dry: bool) -> ApiResult {
        let snap = self.fresh()?;
        let (_, v) = self.find_volume(&snap, id)?;
        let mut path = v["mountPoint"].clone();
        let mut mounted = false;
        if !truth(&path) {
            self.volume_checks(&snap, "open", &json!({"id":id}))?;
            if dry {
                return Ok(json!({"ok":true,"dryRun":true,"mounted":false,"path":null}));
            }
            path = self.mount(id)?["mountPoint"].clone();
            mounted = true;
        }
        if truth(&path) {
            self.ctx.call(
                "shell.openApp",
                &json!({"app":"explorer","params":{"path":path}}),
            )?;
        }
        Ok(json!({"ok":true,"path":path,"mounted":mounted}))
    }
    pub(super) fn set_label(&self, id: &Value, label: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (d, v) = self.find_volume(&snap, id)?;
        self.refuse(arr(&v["protected"]), "label", false, arr(&v["bootEntries"]))?;
        let (target, label) = self.label_target(&v, label)?;
        let (mut mp, mut error) = (None, None);
        let remount =
            truth(&v["mounted"]) && labels::OFFLINE_LABEL.contains(&string(&v["fsType"]).as_str());
        {
            let _op = self.operation(keys(&snap, &d))?;
            self.volume_checks(&snap, "setLabel", &json!({"id":id,"label":label}))?;
            if remount {
                self.unmount_path(&target, v["name"].as_str().unwrap_or(""))?;
            }
            let result = self.dbus(
                &target,
                I_FS,
                "SetLabel",
                Some(&(label.as_str(), empty()).to_variant()),
                self.timeout(),
                v["name"].as_str().unwrap_or(""),
            );
            if remount {
                (mp, error) = self.mount_new(
                    v["id"].as_str().unwrap_or("/"),
                    v["name"].as_str().unwrap_or(""),
                );
            }
            result?;
        }
        self.done();
        Ok(json!({"ok":true,"label":label,"remounted":remount,"mountPoint":mp,"mountError":error}))
    }
    pub(super) fn format(self: &Arc<Self>, args: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (d, _) = self.find_volume(&snap, &args["id"])?;
        let mut op = self.operation(keys(&snap, &d))?;
        let snap = self.fresh()?;
        let (_, v) = self.volume_checks(&snap, "format", args)?;
        let plan = self.format_args(&snap, args, v["kind"] == "partition")?;
        let was = self.teardown(&v)?;
        self.job_call(
            &mut op,
            volume_paths(&v),
            v["id"].as_str().unwrap_or("/"),
            I_BLOCK,
            "Format",
            &(string(&plan["fs"]), opts(&plan["opts"])).to_variant(),
            v["name"].as_str().unwrap_or(""),
        )?;
        let (mp, merr) = if was || truth(&args["mount"]) {
            self.mount_new(
                v["id"].as_str().unwrap_or("/"),
                v["name"].as_str().unwrap_or(""),
            )
        } else {
            (None, None)
        };
        drop(op);
        self.done();
        Ok(json!({"ok":true,"id":v["id"],"mountPoint":mp,"mountError":merr}))
    }
    pub(super) fn delete(self: &Arc<Self>, args: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (d, _) = self.find_volume(&snap, &args["id"])?;
        let mut op = self.operation(keys(&snap, &d))?;
        let snap = self.fresh()?;
        let (d, v) = self.volume_checks(&snap, "deletePartition", args)?;
        self.teardown(&v)?;
        let mut paths = volume_paths(&v);
        paths.insert(string(&d["block"]));
        self.job_call(
            &mut op,
            paths,
            v["id"].as_str().unwrap_or("/"),
            I_PART,
            "Delete",
            &(opts(&json!({"tear-down":true})),).to_variant(),
            v["name"].as_str().unwrap_or(""),
        )?;
        drop(op);
        self.done();
        Ok(json!({"ok":true}))
    }
    pub(super) fn create(self: &Arc<Self>, args: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let d = self.find_disk(&snap, &args["disk"])?;
        let mut op = self.operation(keys(&snap, &d))?;
        let snap = self.fresh()?;
        let d = self.disk_checks(&snap, "createPartition", args)?;
        let p = self.create_args(&snap, &d, args)?;
        let r = self.job_call(
            &mut op,
            disk_paths(&[d.clone()]),
            d["block"].as_str().unwrap_or("/"),
            I_TABLE,
            "CreatePartitionAndFormat",
            &(
                int(&p["offset"]) as u64,
                int(&p["size"]) as u64,
                string(&p["type"]),
                "",
                opts(&p["opts"]),
                string(&p["fs"]),
                opts(&p["fopts"]),
            )
                .to_variant(),
            d["name"].as_str().unwrap_or(""),
        )?;
        let new = nonempty(&first(&r));
        let (mp, merr) =
            if let Some(new) = new.as_ref().filter(|_| args.get("mount").is_none_or(truth)) {
                self.mount_new(new, d["name"].as_str().unwrap_or(""))
            } else {
                (None, None)
            };
        drop(op);
        self.done();
        Ok(json!({"ok":true,"id":new,"mountPoint":mp,"mountError":merr}))
    }
    pub(super) fn initialize(self: &Arc<Self>, args: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let d = self.find_disk(&snap, &args["disk"])?;
        let mut op = self.operation(keys(&snap, &d))?;
        let snap = self.fresh()?;
        let d = self.disk_checks(&snap, "initialize", args)?;
        for v in arr(&d["volumes"]) {
            self.teardown(v)?;
        }
        let table = args.get("table").cloned().unwrap_or(json!("gpt"));
        self.job_call(
            &mut op,
            disk_paths(&[d.clone()]),
            d["block"].as_str().unwrap_or("/"),
            I_BLOCK,
            "Format",
            &(string(&table), opts(&json!({"tear-down":true}))).to_variant(),
            d["name"].as_str().unwrap_or(""),
        )?;
        drop(op);
        self.done();
        Ok(json!({"ok":true}))
    }
    pub(super) fn remove_drive(&self, args: &Value, power_off: bool) -> ApiResult {
        let snap = self.fresh()?;
        let d = self.find_disk(
            &snap,
            if truth(&args["disk"]) {
                &args["disk"]
            } else {
                &args["id"]
            },
        )?;
        let op = self.operation(keys(&snap, &d))?;
        let snap = self.fresh()?;
        let d = self.disk_checks(
            &snap,
            if power_off { "powerOff" } else { "eject" },
            &json!({"disk":d["id"]}),
        )?;
        for x in siblings(&snap, &d) {
            for v in arr(&x["volumes"]) {
                self.teardown(v)?;
            }
        }
        let method = if !power_off {
            Some("Eject")
        } else if truth(&d["canPowerOff"]) {
            Some("PowerOff")
        } else if truth(&d["ejectable"]) {
            Some("Eject")
        } else {
            None
        };
        if let Some(method) = method {
            self.dbus(
                d["drive"].as_str().unwrap_or("/"),
                I_DRIVE,
                method,
                Some(&(empty(),).to_variant()),
                self.timeout(),
                d["model"].as_str().unwrap_or(""),
            )?;
        }
        drop(op);
        self.done();
        Ok(if power_off {
            json!({"ok":true,"how":match method{Some("PowerOff")=>"poweroff",Some("Eject")=>"eject",_=>"unmounted"}})
        } else {
            json!({"ok":true})
        })
    }
    pub(super) fn unlock(&self, id: &Value, passphrase: &Value, mount: bool) -> ApiResult {
        let snap = self.fresh()?;
        let (_, v) = self.find_volume(&snap, id)?;
        let enc = &v["encrypted"];
        if !truth(enc) {
            return Err(self.err("not_encrypted", "not_encrypted", &[]));
        }
        if truth(&enc["unlocked"]) {
            return Ok(
                json!({"ok":true,"cleartext":enc["cleartext"],"already":true,"mountPoint":v["mountPoint"]}),
            );
        }
        let passphrase = passphrase
            .as_str()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| self.err("bad_args", "passphrase_empty", &[]))?;
        let r = self.dbus(
            v["id"].as_str().unwrap_or("/"),
            I_CRYPT,
            "Unlock",
            Some(&(passphrase, empty()).to_variant()),
            self.timeout(),
            v["name"].as_str().unwrap_or(""),
        )?;
        let clear = nonempty(&first(&r));
        let (mp, merr) = if mount && clear.is_some() {
            self.mount_new(
                v["id"].as_str().unwrap_or("/"),
                v["name"].as_str().unwrap_or(""),
            )
        } else {
            (None, None)
        };
        self.done();
        Ok(json!({"ok":true,"cleartext":clear,"mountPoint":mp,"mountError":merr}))
    }
    pub(super) fn lock(&self, id: &Value) -> ApiResult {
        let snap = self.fresh()?;
        let (d, v) = self.find_volume(&snap, id)?;
        let enc = &v["encrypted"];
        if !truth(enc) {
            return Err(self.err("not_encrypted", "not_encrypted", &[]));
        }
        let hard = model::hard(&v["protected"])
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        self.refuse(&hard, "lock", false, &[])?;
        if !truth(&enc["unlocked"]) {
            return Ok(json!({"ok":true,"already":true}));
        }
        {
            let _op = self.operation(keys(&snap, &d))?;
            self.volume_checks(&snap, "lock", &json!({"id":id}))?;
            self.teardown(&v)?;
        }
        self.done();
        Ok(json!({"ok":true}))
    }
    pub(super) fn smart(&self, id: &Value, refresh: bool) -> ApiResult {
        let snap = self.fresh()?;
        let mut d = self.find_disk(&snap, id)?;
        let mut out = d["smart"].clone();
        if !truth(&out["supported"]) || !truth(&d["drive"]) {
            out["message"] = json!(tr(&self.language(), "no_smart", &[]));
            return Ok(out);
        }
        let iface = if out["kind"] == "nvme" { I_NVME } else { I_ATA };
        if refresh {
            self.dbus(
                d["drive"].as_str().unwrap_or("/"),
                iface,
                "SmartUpdate",
                Some(&(empty(),).to_variant()),
                120000,
                d["model"].as_str().unwrap_or(""),
            )?;
            d = self.find_disk(&self.snapshot(true)?, &d["id"])?;
            out = d["smart"].clone();
        }
        match self.dbus(
            d["drive"].as_str().unwrap_or("/"),
            iface,
            "SmartGetAttributes",
            Some(&(empty(),).to_variant()),
            120000,
            d["model"].as_str().unwrap_or(""),
        ) {
            Ok(r) => {
                if iface == I_NVME {
                    if let (Some(out), Some(extra)) = (
                        out.as_object_mut(),
                        model::nvme_health(&first(&r)).as_object(),
                    ) {
                        out.extend(extra.clone());
                    }
                } else {
                    out["attributes"] = model::ata_attributes(&first(&r));
                }
            }
            Err(e) if e.code == "readonly" => out["details"] = json!(false),
            Err(e) => out["detailsError"] = json!(e.message),
        }
        Ok(out)
    }
}
