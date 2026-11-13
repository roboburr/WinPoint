//! win11-panel: the launcher of the native GTK 4 shell (it execs `win11-shell` from its own directory, D-G70) and,
//! with `--http-only`, that shell's backend: the host's services over the JSON API (lib.rs run_http_only).
fn main() {
    // The panel's backend is the single writer of the own desktop entries' names (contracts C6).
    win11_host::enable_desktop_entries();
    win11_host::run("panel");
}
