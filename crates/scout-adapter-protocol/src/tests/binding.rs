use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AdapterId, AdapterPageOutcome, AdapterPageReceipt, AuthContextHandle, AuthContextId,
    CursorHandle, CursorVaultBinding, FailureReason, NormalizedRecord, ProtocolError,
    RedactionSummary, TargetId,
};

use super::fixtures::{
    adapter_id, auth, continuation_receipt, digest, first_request, next_request, target,
    CURSOR_EXPIRES_AT, CURSOR_ISSUED_AT, OBSERVED_AT,
};

type RequestMutation = Box<dyn Fn(&mut crate::AdapterPageRequest)>;

#[test]
fn first_and_continuation_requests_validate() {
    first_request()
        .validate(&target(), &auth(), OBSERVED_AT)
        .unwrap();

    let binding = CursorVaultBinding::for_next_page(
        &continuation_receipt(),
        CURSOR_ISSUED_AT,
        CURSOR_EXPIRES_AT,
    )
    .unwrap();
    let request = next_request();
    request
        .validate(&target(), &auth(), CURSOR_ISSUED_AT)
        .unwrap();
    binding
        .authorize(&request, &target(), &auth(), CURSOR_ISSUED_AT)
        .unwrap();
}

#[test]
fn coverage_epoch_and_sequence_are_one_based() {
    let mut zero_epoch = first_request();
    zero_epoch.coverage.discovery_epoch = 0;
    assert!(zero_epoch
        .validate(&target(), &auth(), OBSERVED_AT)
        .is_err());

    let mut zero_sequence = first_request();
    zero_sequence.coverage.sequence = 0;
    assert!(zero_sequence
        .validate(&target(), &auth(), OBSERVED_AT)
        .is_err());
}

#[test]
fn cursor_binding_rejects_every_changed_dimension() {
    let binding = CursorVaultBinding::for_next_page(
        &continuation_receipt(),
        CURSOR_ISSUED_AT,
        CURSOR_EXPIRES_AT,
    )
    .unwrap();
    let baseline = next_request();
    let mut mutations: Vec<RequestMutation> = vec![
        Box::new(|request| {
            request.target_id = TargetId::new(format!("target:{}", digest('a'))).unwrap()
        }),
        Box::new(|request| request.target_identity_sha256 = digest('c')),
        Box::new(|request| {
            request.adapter_id = AdapterId::new("clark/github-organization@2").unwrap()
        }),
        Box::new(|request| {
            request.auth_context_handle =
                AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000099").unwrap()
        }),
        Box::new(|request| {
            request.auth_context_id =
                AuthContextId::new(format!("authctx:{}", digest('b'))).unwrap()
        }),
        Box::new(|request| request.coverage.sequence += 1),
        Box::new(|request| request.query.page_size = 99),
        Box::new(|request| request.page_ordinal = 2),
        Box::new(|request| {
            request.cursor_handle =
                Some(CursorHandle::new("cursor:00000000-0000-4000-8000-000000000098").unwrap())
        }),
    ];

    for mutation in &mut mutations {
        let mut request = baseline.clone();
        mutation(&mut request);
        assert!(matches!(
            binding.authorize(&request, &target(), &auth(), CURSOR_ISSUED_AT),
            Err(ProtocolError::CursorBinding { .. })
        ));
    }
}

#[test]
fn expired_cursor_is_rejected() {
    let binding = CursorVaultBinding::for_next_page(
        &continuation_receipt(),
        CURSOR_ISSUED_AT,
        CURSOR_EXPIRES_AT,
    )
    .unwrap();
    assert!(matches!(
        binding.authorize(&next_request(), &target(), &auth(), CURSOR_EXPIRES_AT),
        Err(ProtocolError::CursorBinding { .. })
    ));
}

#[test]
fn cursor_authorization_rechecks_auth_expiry() {
    let binding = CursorVaultBinding::for_next_page(
        &continuation_receipt(),
        CURSOR_ISSUED_AT,
        CURSOR_EXPIRES_AT,
    )
    .unwrap();
    let mut expired_auth = auth();
    expired_auth.expires_at_ms = Some(CURSOR_ISSUED_AT);
    assert!(matches!(
        binding.authorize(&next_request(), &target(), &expired_auth, CURSOR_ISSUED_AT),
        Err(ProtocolError::CursorBinding { .. })
    ));
}

#[test]
fn request_rejects_cursor_on_first_page_and_missing_cursor_later() {
    let mut first = first_request();
    first.cursor_handle =
        Some(CursorHandle::new("cursor:00000000-0000-4000-8000-000000000003").unwrap());
    assert!(first.validate(&target(), &auth(), OBSERVED_AT).is_err());

    let mut later = next_request();
    later.cursor_handle = None;
    assert!(later
        .validate(&target(), &auth(), CURSOR_ISSUED_AT)
        .is_err());
}

#[test]
fn denial_is_a_typed_non_empty_coverage_outcome() {
    let receipt = AdapterPageReceipt::new(
        first_request(),
        target(),
        auth(),
        digest('6'),
        OBSERVED_AT,
        AdapterPageOutcome::Denied {
            reason: FailureReason::AccessDenied,
        },
        Vec::new(),
        None,
        RedactionSummary {
            source_records_seen: 0,
            records_emitted: 0,
            fields_omitted: 0,
            values_rejected: 0,
        },
    )
    .unwrap();
    let encoded = serde_json::to_string(&receipt).unwrap();
    assert!(encoded.contains(r#""status":"denied""#));
    assert!(encoded.contains(r#""reason":"access_denied""#));
}

#[test]
fn terminal_success_cannot_carry_a_cursor() {
    let result = AdapterPageReceipt::new(
        first_request(),
        target(),
        auth(),
        digest('6'),
        OBSERVED_AT,
        AdapterPageOutcome::Succeeded { final_page: true },
        Vec::new(),
        Some(CursorHandle::random()),
        RedactionSummary::default(),
    );
    assert!(result.is_err());
    assert_eq!(adapter_id().as_str(), "clark/github-organization@1");
}

#[test]
fn receipt_enforces_redaction_counts_and_response_limits() {
    let mut bad_summary = continuation_receipt();
    bad_summary.redaction_summary.records_emitted = 0;
    assert!(bad_summary.validate_at(CURSOR_ISSUED_AT).is_err());

    let mut too_small = continuation_receipt();
    too_small.request.limits.max_response_bytes = 1;
    assert!(too_small.validate_at(CURSOR_ISSUED_AT).is_err());
}

#[test]
fn record_identity_authority_is_independent_from_query_authorization() {
    let record = NormalizedRecord::new(
        adapter_id(),
        "github".to_owned(),
        "github.repository".to_owned(),
        "global".to_owned(),
        "github-repository:42".to_owned(),
        Some("code_repository".to_owned()),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeSet::new(),
    )
    .unwrap();
    let receipt = AdapterPageReceipt::new(
        first_request(),
        target(),
        auth(),
        digest('6'),
        OBSERVED_AT,
        AdapterPageOutcome::Succeeded { final_page: true },
        vec![record],
        None,
        RedactionSummary {
            source_records_seen: 1,
            records_emitted: 1,
            fields_omitted: 0,
            values_rejected: 0,
        },
    )
    .unwrap();
    assert_eq!(receipt.request.query.authority_scope, "acme");
    assert_eq!(receipt.records[0].identity_authority_scope, "global");
}
