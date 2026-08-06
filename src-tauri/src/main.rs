// Clark owns an in-app terminal. Launching the GUI must never create a second,
// OS-owned console window, including in QA and development builds.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let _diagnostics_guard = clark_desktop_lib::init_diagnostics();
    if clark_desktop_lib::run_signed_computer_use_smoke_if_requested() {
        return;
    }
    if clark_desktop_lib::run_windows_console_smoke_if_requested() {
        return;
    }
    if clark_desktop_lib::run_windows_sandbox_smoke_if_requested() {
        return;
    }
    clark_desktop_lib::run();
}
