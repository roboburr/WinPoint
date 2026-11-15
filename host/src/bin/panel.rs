//! win11-panel: the launcher of the native GTK 4 shell (it execs `win11-shell` from its own directory, D-G70) and,
//! with `--http-only`, that shell's backend: the host's services over the JSON API (lib.rs run_http_only).
fn main() {
    // For people only: tools read the version from the file (lib.rs VERSION_LINE), never by running a binary,
    // because `run` execs win11-shell first, which starts or activates the shell (D-G74a).
    if std::env::args().nth(1).as_deref() == Some("--version") {
        print!("{}", win11_host::VERSION_LINE);
        return;
    }
    // The panel's backend is the single writer of the own desktop entries' names (contracts C6).
    win11_host::enable_desktop_entries();
    win11_host::run("panel");
}
