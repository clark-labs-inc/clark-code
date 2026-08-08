// The desktop owns an in-app terminal. Launching the GUI must never create a second,
// OS-owned console window, including in QA and development builds.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let _diagnostics_guard = desktop_foundation::init_diagnostics();
    if desktop_foundation::run_signed_computer_use_smoke_if_requested() {
        return;
    }
    if desktop_foundation::run_windows_console_smoke_if_requested() {
        return;
    }
    if desktop_foundation::run_windows_sandbox_smoke_if_requested() {
        return;
    }
    desktop_foundation::run_with_product_and_context(
        std::sync::Arc::new(desktop_foundation::product::NeutralProduct),
        tauri::generate_context!(),
    );
}
