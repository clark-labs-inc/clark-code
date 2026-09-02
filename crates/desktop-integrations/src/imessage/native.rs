#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::CString;
    unsafe extern "C" {
        fn cc_integrations_initialize();
        fn cc_integrations_epoch() -> u64;
        fn cc_integrations_interactive() -> bool;
        fn cc_integrations_confirm(
            title: *const std::ffi::c_char,
            body: *const std::ffi::c_char,
            button: *const std::ffi::c_char,
        ) -> bool;
        fn cc_integrations_settings();
    }
    pub fn initialize() {
        unsafe { cc_integrations_initialize() }
    }
    pub fn epoch() -> u64 {
        unsafe { cc_integrations_epoch() }
    }
    pub fn interactive() -> bool {
        unsafe { cc_integrations_interactive() }
    }
    pub fn confirm(title: &str, body: &str, button: &str) -> bool {
        let (Ok(title), Ok(body), Ok(button)) = (
            CString::new(title),
            CString::new(body),
            CString::new(button),
        ) else {
            return false;
        };
        unsafe { cc_integrations_confirm(title.as_ptr(), body.as_ptr(), button.as_ptr()) }
    }
    pub fn open_privacy_settings() {
        unsafe { cc_integrations_settings() }
    }
}

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(not(target_os = "macos"))]
mod unsupported {
    pub fn initialize() {}
    pub fn epoch() -> u64 {
        0
    }
    pub fn interactive() -> bool {
        false
    }
    pub fn confirm(_: &str, _: &str, _: &str) -> bool {
        false
    }
    pub fn open_privacy_settings() {}
}

#[cfg(not(target_os = "macos"))]
pub use unsupported::*;
