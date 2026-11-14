//! win11-installer: the launcher of the native App Installer (reviewed installation of downloaded Linux application
//! packages); it execs `win11-gtk-installer` from its own directory with its arguments (lib.rs exec_native_app).
fn main() {
    win11_host::run("installer");
}
