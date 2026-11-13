use super::*;
use serde_json::json;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};
// This file is compiled into the library and into the win11-launch binary (bin/launch.rs includes
// launch_context.rs by #[path]); the binary has no crate::events, so the fixture keeps its own nonce.
fn nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
struct Fixture {
    root: PathBuf,
    prefix: PathBuf,
    data: PathBuf,
    config: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("win11-portal-optin-{}", nonce()));
        let prefix = root.join("prefix with $ percent% quote\" slash\\");
        let data = root.join("data");
        let config = root.join("config");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::copy("/usr/bin/true", prefix.join("bin/win11-file-portal")).unwrap();
        fs::set_permissions(
            prefix.join("bin/win11-file-portal"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let result = Self {
            root,
            prefix,
            data,
            config,
        };
        result.write();
        result
    }
    fn payload(&self) -> PathBuf {
        self.prefix.join("share/win11-gnome")
    }
    fn write(&self) {
        fs::create_dir_all(self.payload()).unwrap();
        fs::write(
            self.payload().join(".win11-gnome-owned"),
            b"win11-gnome-rust-host-v1\n",
        )
        .unwrap();
        let binary = self.prefix.join("bin/win11-file-portal");
        let quoted = glib::shell_quote(&binary)
            .to_string_lossy()
            .replace('\\', "\\\\");
        let escaped = binary
            .to_str()
            .unwrap()
            .replace('\\', r"\\")
            .replace('"', r#"\""#)
            .replace('$', "$$")
            .replace('%', "%%");
        let contents = [
            (
                "definition",
                self.data
                    .join("xdg-desktop-portal/portals/win11shell.portal"),
                format!("[portal]\nDBusName={NAME}\nInterfaces={IFACE};\n"),
            ),
            (
                "activation",
                self.data
                    .join("dbus-1/services")
                    .join(format!("{NAME}.service")),
                format!("[D-BUS Service]\nName={NAME}\nExec={quoted}\nSystemdService={UNIT}\n"),
            ),
            (
                "unit",
                self.config.join("systemd/user").join(UNIT),
                format!(
                    "[Unit]\nDescription=Fixture\n[Service]\nType=dbus\nBusName={NAME}\nExecStart=/usr/bin/env -- \"{escaped}\"\nSlice=app.slice\n"
                ),
            ),
            (
                "selector",
                self.config.join("xdg-desktop-portal/gnome-portals.conf"),
                format!("[preferred]\ndefault=gnome;gtk;\n{IFACE}=win11shell\n"),
            ),
        ];
        let mut files = serde_json::Map::new();
        for (key, path, contents) in contents {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents.as_bytes()).unwrap();
            files.insert(
                key.into(),
                json!({"path":path,"owned":glib::base64_encode(contents.as_bytes()).to_string()}),
            );
        }
        fs::write(
            self.payload().join("file-portal.json"),
            json!({"marker":"win11-file-portal-v1","files":files}).to_string(),
        )
        .unwrap();
    }
    fn present(&self) -> bool {
        registration_at(&self.prefix, &self.data, &self.config)
    }
}
#[test]
fn owned_registration_accepts_literal_quoted_prefix_and_unrelated_selector_edits() {
    let f = Fixture::new();
    assert!(f.present());
    let selector = f.config.join("xdg-desktop-portal/gnome-portals.conf");
    let mut raw = fs::read_to_string(&selector).unwrap();
    raw.push_str("\n[Other]\nadded=user\n");
    fs::write(selector, raw).unwrap();
    assert!(f.present());
}
#[test]
fn absent_edited_or_symbolic_managed_files_disable_opt_in() {
    let f = Fixture::new();
    for path in [
        f.payload().join(".win11-gnome-owned"),
        f.payload().join("file-portal.json"),
        f.data.join("xdg-desktop-portal/portals/win11shell.portal"),
        f.data
            .join("dbus-1/services")
            .join(format!("{NAME}.service")),
        f.config.join("systemd/user").join(UNIT),
    ] {
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(!f.present());
        fs::write(&path, b"unowned").unwrap();
        assert!(!f.present());
        fs::remove_file(&path).unwrap();
        let target = f.root.join("symlink-target");
        fs::write(&target, &original).unwrap();
        symlink(&target, &path).unwrap();
        assert!(!f.present());
        fs::remove_file(&path).unwrap();
        fs::write(path, original).unwrap();
        assert!(f.present());
    }
}
#[test]
fn backend_selection_and_native_executable_are_required() {
    let f = Fixture::new();
    let selector = f.config.join("xdg-desktop-portal/gnome-portals.conf");
    fs::write(&selector, format!("[preferred]\n{IFACE}=gtk;\n")).unwrap();
    assert!(!f.present());
    f.write();
    let binary = f.prefix.join("bin/win11-file-portal");
    fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
    assert!(!f.present());
    fs::copy("/usr/bin/true", &binary).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!f.present());
}
#[test]
fn malformed_or_redirected_ownership_cannot_enable_opt_in() {
    let f = Fixture::new();
    let path = f.payload().join("file-portal.json");
    let original = fs::read(&path).unwrap();
    for change in ["marker", "path", "base64"] {
        let mut v: Value = serde_json::from_slice(&original).unwrap();
        match change {
            "marker" => v["marker"] = json!("foreign"),
            "path" => v["files"]["activation"]["path"] = json!("/tmp/foreign.service"),
            _ => v["files"]["activation"]["owned"] = json!("@@invalid-base64@@"),
        };
        fs::write(&path, v.to_string()).unwrap();
        assert!(!f.present());
    }
    fs::write(&path, vec![b' '; 512 * 1024 + 1]).unwrap();
    assert!(!f.present());
}
#[test]
#[ignore = "run with a private HOME/XDG/prefix and explicit expected GTK launch setting"]
fn isolated_launch_environment() {
    use gio::prelude::*;
    use std::os::unix::ffi::OsStrExt;
    let expected = std::env::var_os("WIN11_PORTAL_EXPECT_GTK").unwrap();
    let expected = if expected == "__unset__" {
        None
    } else {
        Some(expected.as_bytes().to_vec())
    };
    let check = |rows: Vec<Vec<u8>>| {
        let gtk = rows
            .iter()
            .find_map(|row| row.strip_prefix(b"GTK_USE_PORTAL=").map(Vec::from));
        assert_eq!(gtk, expected);
        assert!(!rows.iter().any(|v| v.starts_with(b"WIN11_PORTAL_ENV=")));
        assert!(
            !rows
                .iter()
                .any(|v| v.starts_with(b"WIN11_PORTAL_PARENT_GTK_USE_PORTAL="))
        );
    };
    let context = crate::launch_context::new(None);
    check(
        context
            .environment()
            .iter()
            .map(|v| v.as_bytes().to_vec())
            .collect(),
    );
    let mut command = Command::new("/usr/bin/env");
    command.arg("-0");
    crate::launch_context::configure_portal_child_environment(&mut command, None);
    let output = command.output().unwrap();
    assert!(output.status.success());
    check(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(Vec::from)
            .collect(),
    );
    if captured().is_none() {
        let mut explicit = Command::new("/usr/bin/env");
        explicit.arg("-0").env("GTK_USE_PORTAL", "0");
        crate::launch_context::configure_portal_child_environment(&mut explicit, None);
        let output = explicit.output().unwrap();
        assert!(
            output
                .stdout
                .split(|b| *b == 0)
                .any(|v| v == b"GTK_USE_PORTAL=0")
        );
        let base = vec![(OsString::from("GTK_USE_PORTAL"), OsString::from("0"))];
        let mut explicit = Command::new("/usr/bin/env");
        explicit.arg("-0").env_clear().envs(base.clone());
        crate::launch_context::configure_portal_child_environment(&mut explicit, Some(&base));
        assert_eq!(explicit.output().unwrap().stdout, b"GTK_USE_PORTAL=0\0");
    }
}
