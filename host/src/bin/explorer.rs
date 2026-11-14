//! win11-explorer: the launcher of native File Explorer (the inode/directory handler, `--tab-ticket` and the desktop
//! entry); it execs `win11-gtk-explorer` from its own directory with its arguments (lib.rs exec_native_app).
fn main() {
    win11_host::run("explorer");
}
