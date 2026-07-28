use std::collections::{BTreeMap, BTreeSet};

use crate::{AdapterPageReceipt, CursorVaultBinding, NormalizedRecord, SafeFieldValue};

use super::fixtures::{
    adapter_id, continuation_receipt, digest, query, record, CURSOR_EXPIRES_AT, CURSOR_ISSUED_AT,
};

#[test]
fn credential_shaped_values_are_rejected_before_record_creation() {
    let protected = [
        "Bearer definitely-not-safe",
        "ghp_abcdefghijklmnopqrstuvwxyz1234567890",
        "AKIAABCDEFGHIJKLMNOP",
        "password=hunter2",
        "https://user:secret@example.com/path",
        "eyJhbGciOiJSUzI1NiJ9.aaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ];
    for value in protected {
        let result = NormalizedRecord::new(
            adapter_id(),
            "github".to_owned(),
            "github.repository".to_owned(),
            "acme".to_owned(),
            "repo:clark".to_owned(),
            None,
            BTreeSet::new(),
            BTreeMap::from([(
                "description".to_owned(),
                SafeFieldValue::Text(value.to_owned()),
            )]),
            BTreeSet::new(),
        );
        assert!(result.is_err(), "accepted protected value: {value}");
    }
}

#[test]
fn provider_cursor_field_names_are_reserved() {
    for name in ["cursor", "next_token", "next-page-token", "page_token"] {
        let result = NormalizedRecord::new(
            adapter_id(),
            "github".to_owned(),
            "github.repository".to_owned(),
            "acme".to_owned(),
            "repo:clark".to_owned(),
            None,
            BTreeSet::new(),
            BTreeMap::from([(
                name.to_owned(),
                SafeFieldValue::Text("opaque-provider-value".to_owned()),
            )]),
            BTreeSet::new(),
        );
        assert!(result.is_err(), "accepted protected field: {name}");
    }
}

#[test]
fn serialized_contract_contains_only_opaque_cursor_handles() {
    let receipt = continuation_receipt();
    let binding =
        CursorVaultBinding::for_next_page(&receipt, CURSOR_ISSUED_AT, CURSOR_EXPIRES_AT).unwrap();
    for encoded in [
        serde_json::to_value(&receipt).unwrap(),
        serde_json::to_value(&binding).unwrap(),
    ] {
        assert_cursor_values_are_handles(&encoded);
        assert_no_protected_keys(&encoded);
        let text = serde_json::to_string(&encoded).unwrap();
        assert!(!text.contains("raw_provider_cursor"));
        assert!(!text.contains("credentials"));
        assert!(!text.contains("secret_access_key"));
    }
    let receipt_json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        receipt_json["records"][0]["provider_namespace"],
        serde_json::json!("github")
    );
}

#[test]
fn unknown_wire_fields_are_rejected() {
    let mut value = serde_json::to_value(continuation_receipt()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("provider_response".to_owned(), serde_json::json!("unsafe"));
    assert!(serde_json::from_value::<AdapterPageReceipt>(value).is_err());

    let mut nested = serde_json::to_value(continuation_receipt()).unwrap();
    nested["outcome"]["provider_message"] = serde_json::json!("unsafe");
    assert!(serde_json::from_value::<AdapterPageReceipt>(nested).is_err());
}

#[test]
fn fingerprints_are_deterministic_and_tamper_evident() {
    let first = record();
    let mut second = record();
    assert_eq!(
        first.fingerprint_sha256().unwrap(),
        second.fingerprint_sha256().unwrap()
    );
    second.fields.insert(
        "name".to_owned(),
        SafeFieldValue::Text("changed".to_owned()),
    );
    assert!(second.validate().is_err());

    let mut receipt = continuation_receipt();
    receipt.redaction_summary.fields_omitted += 1;
    assert!(receipt.validate_at(CURSOR_ISSUED_AT).is_err());

    let mut reordered = query();
    reordered.filters = reordered.filters.into_iter().rev().collect();
    assert_eq!(
        query().fingerprint_sha256().unwrap(),
        reordered.fingerprint_sha256().unwrap()
    );
    assert_eq!(digest('f').len(), 64);
}

fn assert_cursor_values_are_handles(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if key.contains("cursor") {
                    match value {
                        serde_json::Value::String(handle) => {
                            assert!(handle.starts_with("cursor:"));
                            assert_eq!(handle.len(), "cursor:".len() + 36);
                        }
                        serde_json::Value::Null => {}
                        _ => panic!("cursor field was not an opaque handle"),
                    }
                }
                assert_cursor_values_are_handles(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_cursor_values_are_handles(value);
            }
        }
        _ => {}
    }
}

fn assert_no_protected_keys(value: &serde_json::Value) {
    const PROTECTED: &[&str] = &[
        "authorization",
        "cookie",
        "credentials",
        "access_key",
        "secret_access_key",
        "api_key",
        "password",
        "private_key",
        "refresh_token",
        "access_token",
        "id_token",
        "session_token",
        "next_token",
        "page_token",
        "provider_response",
    ];
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                assert!(!PROTECTED.contains(&key.as_str()), "protected key: {key}");
                assert_no_protected_keys(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_protected_keys(value);
            }
        }
        _ => {}
    }
}
