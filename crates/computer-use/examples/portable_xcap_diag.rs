#[cfg(all(
    feature = "helper-service",
    any(target_os = "linux", target_os = "windows")
))]
fn main() {
    match xcap::Window::all() {
        Ok(windows) => {
            for window in windows {
                println!(
                    "{:?}",
                    (
                        window.id(),
                        window.pid(),
                        window.app_name(),
                        window.title(),
                        window.x(),
                        window.y(),
                        window.width(),
                        window.height(),
                        window.z(),
                        window.is_minimized(),
                    )
                );
            }
        }
        Err(error) => {
            eprintln!("xcap window enumeration failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(all(
    feature = "helper-service",
    any(target_os = "linux", target_os = "windows")
)))]
fn main() {
    eprintln!("portable_xcap_diag requires --features helper-service on Windows or Linux");
    std::process::exit(2);
}
