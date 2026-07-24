#![cfg(target_os = "macos")]

use computer_use::{native_backend, ComputerUseError};

#[test]
fn native_backend_fails_closed_outside_a_signed_packaged_app() {
    let backend = native_backend().expect("native macOS backend");
    match backend.permissions() {
        Err(ComputerUseError::HelperUnavailable(_)) => {}
        Err(ComputerUseError::HelperRejected(message)) => {
            assert!(
                message.contains("parent must be a valid Clark Code")
                    || message.contains("approved Clark team"),
                "unexpected signed-boundary rejection: {message}"
            );
        }
        other => panic!("unsigned or unpackaged callers must fail closed: {other:?}"),
    }
}
