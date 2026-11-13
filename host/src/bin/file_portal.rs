//! D-Bus-activated Explorer file selection backend.
fn main() {
    if let Err(error) = win11_host::run_file_portal() {
        eprintln!("Win11 file portal: {error}");
        std::process::exit(1);
    }
}
