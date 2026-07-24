// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if clark_desktop_lib::run_signed_computer_use_smoke_if_requested() {
        return;
    }
    clark_desktop_lib::run();
}
