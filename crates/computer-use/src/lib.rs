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
    #[cfg(not(target_os = "macos"))]
    {
        Err(ComputerUseError::UnsupportedPlatform(
            std::env::consts::OS.to_string(),
        ))
    }
}

#[doc(hidden)]
#[cfg(all(target_os = "macos", feature = "helper-service"))]
pub fn run_native_helper(
    ipc_fd: i32,
    control_fd: i32,
    data_dir: std::path::PathBuf,
) -> Result<(), ComputerUseError> {
    macos::helper::run(ipc_fd, control_fd, data_dir)
}

#[doc(hidden)]
#[cfg(all(target_os = "macos", feature = "helper-service"))]
pub fn native_helper_self_test() -> Result<(), ComputerUseError> {
    macos::auth::verify_helper_signature().map_err(ComputerUseError::HelperRejected)
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
