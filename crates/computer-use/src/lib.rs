//! Native computer-use boundary for Clark Code.
//!
//! The provider-facing tool layer depends only on [`ComputerBackend`]. Tests
//! use [`SimulatedComputerBackend`]; the shipped macOS build uses
//! [`native_backend`]. Every action is bound to a concrete window identity and
//! requires a fresh observation, so model-supplied element ids cannot silently
//! drift to another app or a later UI state.

mod action;
mod approval_store;
mod lease;
mod observation;
mod policy;
mod simulator;
mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod portable;

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn suppress_portable_console_window(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(PORTABLE_CHILD_CREATION_FLAGS);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(windows)]
const PORTABLE_CHILD_CREATION_FLAGS: u32 = 0x0800_0000;

pub use action::{
    ActionAuthorization, ActionDisposition, ActionKind, ActionLocation, ActionReceipt, AppApproval,
    ApplicationIdentity, ApprovalSnapshot, CancelAck, ComputerAction, PrepareActionRequest,
    PreparedAction, ReceiptOutcome, RedactedActionPreview, TrustedActionAssessment,
};
pub use approval_store::{default_approval_store, ApprovalStore};
pub use observation::{AccessibilityDiff, ElementChange, ObservationSettlement, ValueConstraints};
pub use policy::{
    assess_click, assess_keypress, assess_proposed_action, assess_type_text, ensure_bundle_allowed,
    ensure_window_allowed, validate_intent, validate_intent_shape,
};
pub use simulator::SimulatedComputerBackend;
pub use types::{
    ActionIntent, ActionRisk, ClickRequest, ComputerBackend, ComputerUseError, ElementInfo, Key,
    KeyPressRequest, Modifier, MouseButton, Observation, PermissionRequest, PermissionStatus,
    Point, Rect, RiskAssessment, Screenshot, TypeTextRequest, WindowFilter, WindowInfo,
    WindowTarget,
};

/// Construct the native backend for this host.
pub fn native_backend() -> Result<std::sync::Arc<dyn ComputerBackend>, ComputerUseError> {
    #[cfg(target_os = "macos")]
    {
        Ok(std::sync::Arc::new(macos::client::MacHelperBackend::new()?))
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        Ok(std::sync::Arc::new(
            portable::client::PortableServiceBackend::new()?,
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(ComputerUseError::UnsupportedPlatform(
            std::env::consts::OS.to_string(),
        ))
    }
}

#[doc(hidden)]
#[cfg(all(target_os = "macos", feature = "helper-service"))]
pub fn run_native_helper(
    socket_path: std::path::PathBuf,
    data_dir: std::path::PathBuf,
) -> Result<(), ComputerUseError> {
    macos::helper::run(socket_path, data_dir)
}

#[doc(hidden)]
#[cfg(all(target_os = "macos", feature = "helper-service"))]
pub fn native_helper_self_test() -> Result<(), ComputerUseError> {
    macos::auth::verify_service_signature().map_err(ComputerUseError::HelperRejected)
}

#[doc(hidden)]
#[cfg(all(
    any(target_os = "linux", target_os = "windows"),
    feature = "helper-service"
))]
pub fn run_portable_service(
    socket_name: String,
    data_dir: std::path::PathBuf,
    client_pid: u32,
) -> Result<(), ComputerUseError> {
    portable::service::run(socket_name, data_dir, client_pid)
}

#[doc(hidden)]
#[cfg(all(
    any(target_os = "linux", target_os = "windows"),
    feature = "helper-service"
))]
pub fn portable_service_self_test() -> Result<(), ComputerUseError> {
    portable::auth::verify_own_executable()
}

pub(crate) fn encode_rgba_png(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Vec<u8>, ComputerUseError> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(ComputerUseError::Os(format!(
            "invalid RGBA frame: got {} bytes for {width}x{height}, expected {expected}",
            rgba.len()
        )));
    }
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| ComputerUseError::Os(format!("PNG header failed: {error}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| ComputerUseError::Os(format!("PNG encoding failed: {error}")))?;
    }
    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoder_emits_a_real_png() {
        let bytes = encode_rgba_png(1, 1, &[12, 34, 56, 255]).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
