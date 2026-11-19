//! The system programs the Settings services start (notes/claude/settings-parity.md §6.6): pkexec with GNOME's own
//! helpers, grdctl, tecla, bluetooth-sendto, passwd, nmcli, localedef, ubuntu-insights, simple-scan.
//!
//! Every new spawn resolves its program through [`system_tool`]: `/usr/bin/<name>`, or `$WIN11_SYSTEM_TOOL_DIR/<name>`
//! in the test sandboxes (tests/gnome/harness.py and tests/rust-sandbox/run-tests.sh point it at their stub
//! directories). The harness's exec rule can then see that no real pkexec, passwd or nmcli ran from a WinPoint
//! process. A set but unusable directory (relative, missing, not ours, group- or world-writable) refuses every tool
//! instead of falling back to /usr/bin.
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The test seam: a private directory of stubs that replaces /usr/bin for [`system_tool`].
pub(crate) const TOOL_DIR_VAR: &str = "WIN11_SYSTEM_TOOL_DIR";

/// A program name: letters, digits, `.`, `_`, `+`, `-`, at most 64, not starting with `.` or `-`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// A directory that only this user can change.
fn private_dir(dir: &Path) -> bool {
    dir.is_absolute()
        && std::fs::symlink_metadata(dir).is_ok_and(|m| {
            m.is_dir()
                && m.uid() == unsafe { libc::geteuid() }
                && m.permissions().mode() & 0o022 == 0
        })
}

fn resolve(name: &str, seam: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if !valid_name(name) {
        return None;
    }
    match seam {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            private_dir(&dir).then(|| dir.join(name))
        }
        None => Some(Path::new("/usr/bin").join(name)),
    }
}

/// The path of a system program: `/usr/bin/<name>`, or the sandbox's stub (module docs). None for a bad name or an
/// unusable seam directory.
#[allow(dead_code)] // the batches' spawns
pub(crate) fn system_tool(name: &str) -> Option<PathBuf> {
    resolve(name, std::env::var_os(TOOL_DIR_VAR))
}

/// A GNOME helper that runs through pkexec (`/usr/libexec/<name>`): only ever an argument of the pkexec that
/// [`system_tool`] resolves, so the sandbox's pkexec stub logs it and nothing runs it.
#[allow(dead_code)] // A (set-debian-keyboard), C (gnome-remote-desktop-enable-service)
pub(crate) fn libexec(name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| Path::new("/usr/libexec").join(name))
}

/// A Command for a system program, with LC_ALL=C so its output and prompts can be parsed.
#[allow(dead_code)] // the batches' spawns
pub(crate) fn command(name: &str) -> Option<Command> {
    let path = system_tool(name)?;
    let mut command = Command::new(path);
    command.env("LC_ALL", "C");
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_come_from_usr_bin_or_a_private_stub_directory() {
        assert_eq!(
            resolve("pkexec", None),
            Some(PathBuf::from("/usr/bin/pkexec"))
        );
        for bad in [
            "",
            "../bin/sh",
            "a/b",
            "-rf",
            ".hidden",
            "x y",
            &"x".repeat(65),
        ] {
            assert_eq!(resolve(bad, None), None, "{bad:?}");
        }
        let root = std::env::temp_dir().join(format!("tools-{}", crate::events::nonce()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve("grdctl", Some(root.clone().into())),
            Some(root.join("grdctl"))
        );
        // a seam that is not private refuses every tool, never /usr/bin
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(resolve("grdctl", Some(root.clone().into())), None);
        assert_eq!(resolve("grdctl", Some("relative/stubs".into())), None);
        assert_eq!(resolve("grdctl", Some(root.join("missing").into())), None);
        assert_eq!(
            libexec("gnome-control-center-set-debian-keyboard").unwrap(),
            { PathBuf::from("/usr/libexec/gnome-control-center-set-debian-keyboard") }
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
