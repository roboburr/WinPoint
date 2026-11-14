//! win11-default-apps: the launcher of native Default apps; it execs `win11-gtk-defaultapps` from its own directory
//! (lib.rs exec_native_app), which starts `win11-app --app defaultapps --http-only` as its backend.
fn main() {
    win11_host::run("defaultapps");
}
