use exec_core::{MAX_EXEC_PROTOCOL_MESSAGE_BYTES, MAX_TARGET_SERVICE_REQUEST_BYTES};
use exec_protocol::{method, Request, TargetServiceParams};

use super::{checked_fs_path, target_service_request_encoded_len_allowed, Config};

fn base64_encoded_len(decoded_len: usize) -> usize {
    decoded_len.div_ceil(3) * 4
}

#[test]
fn target_service_base64_gate_allows_exact_cap_and_rejects_next_byte() {
    // Keep the 72 MiB boundary test allocation-free. The loopback integration
    // test separately sends one real request above tungstenite's 16 MiB default.
    let encoded_at_cap = base64_encoded_len(MAX_TARGET_SERVICE_REQUEST_BYTES);
    let encoded_over_cap = base64_encoded_len(MAX_TARGET_SERVICE_REQUEST_BYTES + 1);

    assert!(target_service_request_encoded_len_allowed(encoded_at_cap));
    assert!(!target_service_request_encoded_len_allowed(
        encoded_over_cap
    ));
    assert_eq!(encoded_at_cap, 96 * 1024 * 1024);
    assert_eq!(encoded_over_cap, encoded_at_cap + 4);
}

#[test]
fn protocol_message_limit_has_room_for_the_capped_service_envelope() {
    let empty_request = Request::new(
        1,
        method::TARGET_SERVICE_CALL,
        serde_json::to_value(TargetServiceParams {
            service: "scout-store-v1".into(),
            root: "/configured/remote/root/scout".into(),
            request: String::new(),
        })
        .unwrap(),
    );
    let envelope_without_payload = serde_json::to_vec(&empty_request).unwrap().len();
    let message_at_cap =
        envelope_without_payload + base64_encoded_len(MAX_TARGET_SERVICE_REQUEST_BYTES);

    assert!(message_at_cap < MAX_EXEC_PROTOCOL_MESSAGE_BYTES);
}

#[cfg(unix)]
#[test]
fn canonical_root_alias_is_allowed_without_permitting_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real_root = temp.path().join("real-root");
    let alias_root = temp.path().join("alias-root");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(real_root.join("inside")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&real_root, &alias_root).unwrap();
    symlink(&outside, real_root.join("escape")).unwrap();
    let config = Config {
        token: "test".into(),
        root: Some(alias_root),
        home: None,
        addr: "127.0.0.1:0".into(),
    };

    let canonical_inside = real_root.canonicalize().unwrap().join("inside");
    assert_eq!(
        checked_fs_path(canonical_inside.to_str().unwrap(), &config).unwrap(),
        canonical_inside
    );

    let escaped = real_root.join("escape");
    assert!(checked_fs_path(escaped.to_str().unwrap(), &config).is_err());
}
