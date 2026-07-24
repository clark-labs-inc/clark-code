use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

use crate::{PermissionRequest, PermissionStatus};

pub fn preflight() -> PermissionStatus {
    PermissionStatus {
        accessibility: ax_trusted(false),
        screen_recording: unsafe { ffi::CGPreflightScreenCaptureAccess() },
        screen_recording_restart_required: false,
    }
}

pub fn request(request: PermissionRequest) {
    if request.screen_recording && !unsafe { ffi::CGPreflightScreenCaptureAccess() } {
        unsafe {
            ffi::CGRequestScreenCaptureAccess();
        }
    }
    if request.accessibility && !ax_trusted(false) {
        ax_trusted(true);
    }
}

fn ax_trusted(prompt: bool) -> bool {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::from(prompt);
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    unsafe { accessibility_sys::AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

mod ffi {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGPreflightScreenCaptureAccess() -> bool;
        pub fn CGRequestScreenCaptureAccess() -> bool;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preflight_is_side_effect_free() {
        let _ = super::preflight();
    }
}
