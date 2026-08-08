use scout_adapter_protocol::{AdapterId, NormalizedLink, SafeFieldValue};
use serde_json::json;

use crate::{
    normalize_json, CapsuleError, CapsuleLimits, CapsuleResponse, CAPSULE_ABI_VERSION,
    RESPONSE_SCHEMA,
};

fn candidate_request() -> serde_json::Value {
    json!({
        "abi_version": CAPSULE_ABI_VERSION,
        "schema": "scout-capsule-request-v1",
        "adapter_id": "clark/aws@1",
        "provider_namespace": "aws",
        "records": [
            {
                "provider_type": "aws.ec2.instance",
                "identity_authority_scope": "123456789012",
                "native_id": "i-0123456789abcdef0",
                "semantic_kind": "compute_instance",
                "labels": ["production", "linux", "production"],
                "fields": [
                    {
                        "name": "state",
                        "value": {"kind": "text", "value": "running"}
                    },
                    {
                        "name": "vcpus",
                        "value": {"kind": "unsigned", "value": 8}
                    }
                ],
                "links": [
                    {
                        "relationship_type": "owned_by",
                        "target_provider_namespace": "aws",
                        "target_provider_type": "aws.account",
                        "target_authority_scope": "global",
                        "target_native_id": "123456789012",
                        "qualifier": null
                    },
                    {
                        "relationship_type": "owned_by",
                        "target_provider_namespace": "aws",
                        "target_provider_type": "aws.account",
                        "target_authority_scope": "global",
                        "target_native_id": "123456789012",
                        "qualifier": null
                    }
                ]
            }
        ]
    })
}

fn normalize(value: &serde_json::Value) -> CapsuleResponse {
    let bytes = serde_json::to_vec(value).unwrap();
    let output = normalize_json(&bytes, CapsuleLimits::default()).unwrap();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn repeated_invocations_are_byte_identical() {
    let bytes = serde_json::to_vec(&candidate_request()).unwrap();
    let first = normalize_json(&bytes, CapsuleLimits::default()).unwrap();
    let second = normalize_json(&bytes, CapsuleLimits::default()).unwrap();

    assert_eq!(first, second);
    let response: CapsuleResponse = serde_json::from_slice(&first).unwrap();
    assert_eq!(response.abi_version, CAPSULE_ABI_VERSION);
    assert_eq!(response.schema, RESPONSE_SCHEMA);
}

#[test]
fn canonicalizes_order_and_reports_duplicate_set_members() {
    let mut reordered = candidate_request();
    let mut other = reordered["records"][0].clone();
    other["native_id"] = json!("i-11111111111111111");
    reordered["records"].as_array_mut().unwrap().push(other);
    let first = normalize(&reordered);

    let record = &mut reordered["records"][0];
    record["labels"] = json!(["production", "production", "linux"]);
    record["fields"].as_array_mut().unwrap().reverse();
    record["links"].as_array_mut().unwrap().reverse();
    reordered["records"].as_array_mut().unwrap().reverse();
    let second = normalize(&reordered);

    assert_eq!(
        first.receipt.normalized_page_sha256,
        second.receipt.normalized_page_sha256
    );
    assert_eq!(first.page, second.page);
    assert_eq!(first.receipt.duplicate_labels_removed, 2);
    assert_eq!(first.receipt.duplicate_links_removed, 2);
    assert!(first
        .page
        .records
        .iter()
        .all(|record| record.labels.len() == 2 && record.links.len() == 1));
}

#[test]
fn derives_and_validates_protocol_records_inside_boundary() {
    let response = normalize(&candidate_request());
    let record = &response.page.records[0];

    record.validate().unwrap();
    assert_eq!(
        response.page.adapter_id,
        AdapterId::new("clark/aws@1").unwrap()
    );
    assert_eq!(
        record.fields.get("state"),
        Some(&SafeFieldValue::Text("running".into()))
    );
}

#[test]
fn rejects_protected_payloads_without_echoing_them() {
    let mut request = candidate_request();
    request["records"][0]["fields"][0]["value"]["value"] = json!("AKIA1234567890ABCDEFGH");
    let secret = "AKIA1234567890ABCDEFGH";
    let bytes = serde_json::to_vec(&request).unwrap();
    let error = normalize_json(&bytes, CapsuleLimits::default()).unwrap_err();

    assert!(matches!(
        error,
        CapsuleError::InvalidRequest {
            field: "records",
            ..
        }
    ));
    assert!(!error.to_string().contains(secret));
}

#[test]
fn validation_errors_do_not_echo_untrusted_identity_text() {
    let mut request = candidate_request();
    let private_marker = "private-provider-marker";
    request["provider_namespace"] = json!(private_marker);
    let bytes = serde_json::to_vec(&request).unwrap();
    let error = normalize_json(&bytes, CapsuleLimits::default()).unwrap_err();

    assert!(matches!(error, CapsuleError::InvalidRequest { .. }));
    assert!(!error.to_string().contains(private_marker));
}

#[test]
fn validates_provider_namespace_even_for_an_empty_page() {
    let mut request = candidate_request();
    request["provider_namespace"] = json!("invalid namespace");
    request["records"] = json!([]);
    let bytes = serde_json::to_vec(&request).unwrap();

    assert!(matches!(
        normalize_json(&bytes, CapsuleLimits::default()),
        Err(CapsuleError::InvalidRequest {
            field: "provider_namespace",
            ..
        })
    ));
}

#[test]
fn rejects_duplicate_field_names_instead_of_last_write_wins() {
    let mut request = candidate_request();
    request["records"][0]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "name": "state",
            "value": {"kind": "text", "value": "stopped"}
        }));
    let bytes = serde_json::to_vec(&request).unwrap();

    assert_eq!(
        normalize_json(&bytes, CapsuleLimits::default()).unwrap_err(),
        CapsuleError::Duplicate {
            field: "field name"
        }
    );
}

#[test]
fn rejects_duplicate_normalized_records() {
    let mut request = candidate_request();
    let record = request["records"][0].clone();
    request["records"].as_array_mut().unwrap().push(record);
    let bytes = serde_json::to_vec(&request).unwrap();

    assert_eq!(
        normalize_json(&bytes, CapsuleLimits::default()).unwrap_err(),
        CapsuleError::Duplicate {
            field: "normalized record"
        }
    );
}

#[test]
fn enforces_limits_before_deserialization_and_before_return() {
    let bytes = serde_json::to_vec(&candidate_request()).unwrap();
    let limits = CapsuleLimits {
        max_input_bytes: bytes.len() - 1,
        ..CapsuleLimits::default()
    };
    assert!(matches!(
        normalize_json(&bytes, limits),
        Err(CapsuleError::LimitExceeded {
            resource: "input_bytes",
            ..
        })
    ));

    let limits = CapsuleLimits {
        max_output_bytes: 1,
        ..CapsuleLimits::default()
    };
    assert!(matches!(
        normalize_json(&bytes, limits),
        Err(CapsuleError::LimitExceeded {
            resource: "output_bytes",
            ..
        })
    ));

    let mut request = candidate_request();
    let other = request["records"][0].clone();
    request["records"].as_array_mut().unwrap().push(other);
    let bytes = serde_json::to_vec(&request).unwrap();
    let limits = CapsuleLimits {
        max_records: 1,
        ..CapsuleLimits::default()
    };
    assert!(matches!(
        normalize_json(&bytes, limits),
        Err(CapsuleError::LimitExceeded {
            resource: "records",
            ..
        })
    ));
}

#[test]
fn enforces_depth_structure_and_string_limits() {
    let nested = br#"[[[[[[0]]]]]]"#;
    let limits = CapsuleLimits {
        max_nesting_depth: 5,
        ..CapsuleLimits::default()
    };
    assert_eq!(
        normalize_json(nested, limits).unwrap_err(),
        CapsuleError::LimitExceeded {
            resource: "nesting_depth",
            limit: 5,
            observed: 6
        }
    );

    let structural = br#"[0,1,2]"#;
    let limits = CapsuleLimits {
        max_structural_tokens: 3,
        ..CapsuleLimits::default()
    };
    assert!(matches!(
        normalize_json(structural, limits),
        Err(CapsuleError::LimitExceeded {
            resource: "structural_tokens",
            ..
        })
    ));

    let long_string = br#""abcdef""#;
    let limits = CapsuleLimits {
        max_string_token_bytes: 5,
        ..CapsuleLimits::default()
    };
    assert!(matches!(
        normalize_json(long_string, limits),
        Err(CapsuleError::LimitExceeded {
            resource: "string_token_bytes",
            ..
        })
    ));
}

#[test]
fn malformed_json_error_does_not_include_payload_content() {
    let payload = br#"{"not_secret_but_private":"sensitive-content","#;
    let error = normalize_json(payload, CapsuleLimits::default()).unwrap_err();

    assert!(matches!(error, CapsuleError::MalformedJson { .. }));
    assert!(!error.to_string().contains("sensitive-content"));
}

#[test]
fn typed_link_fixture_remains_wasm_clean() {
    let response = normalize(&candidate_request());
    assert_eq!(
        response.page.records[0].links.iter().next(),
        Some(&NormalizedLink {
            relationship_type: "owned_by".into(),
            target_provider_namespace: "aws".into(),
            target_provider_type: "aws.account".into(),
            target_authority_scope: "global".into(),
            target_native_id: "123456789012".into(),
            qualifier: None,
        })
    );
}

#[test]
fn normalizes_a_full_default_page() {
    let mut request = candidate_request();
    let template = request["records"][0].clone();
    let records = request["records"].as_array_mut().unwrap();
    records.clear();
    for index in 0..CapsuleLimits::default().max_records {
        let mut record = template.clone();
        record["native_id"] = json!(format!("i-{index:017x}"));
        records.push(record);
    }

    let response = normalize(&request);
    assert_eq!(
        response.receipt.normalized_records,
        CapsuleLimits::default().max_records
    );
    assert!(response
        .page
        .records
        .windows(2)
        .all(|pair| pair[0].record_id < pair[1].record_id));
}
