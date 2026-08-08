use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AdapterId, AdapterPageLimits, AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest,
    AdapterQuery, AuthContextDescriptor, AuthContextHandle, AuthSourceKind, CoverageBinding,
    CursorHandle, NormalizedRecord, RedactionSummary, RequestId, SafeFieldValue, TargetIdentity,
};

pub(super) const REQUESTED_AT: u64 = 1_000;
pub(super) const OBSERVED_AT: u64 = 1_100;
pub(super) const CURSOR_ISSUED_AT: u64 = 1_200;
pub(super) const CURSOR_EXPIRES_AT: u64 = 5_000;

pub(super) fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

pub(super) fn adapter_id() -> AdapterId {
    AdapterId::new("clark/github-organization@1").unwrap()
}

pub(super) fn target() -> TargetIdentity {
    TargetIdentity::new(
        digest('1'),
        digest('2'),
        digest('3'),
        digest('4'),
        "linux".to_owned(),
        "x86_64".to_owned(),
    )
    .unwrap()
}

pub(super) fn auth() -> AuthContextDescriptor {
    AuthContextDescriptor::new(
        AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap(),
        target().target_id,
        adapter_id(),
        "github".to_owned(),
        "acme".to_owned(),
        "principal:42".to_owned(),
        AuthSourceKind::CliProfile,
        digest('5'),
        900,
        Some(10_000),
    )
    .unwrap()
}

pub(super) fn coverage() -> CoverageBinding {
    CoverageBinding {
        enterprise_id: "enterprise:acme".to_owned(),
        charter_id: "charter:topology".to_owned(),
        discovery_epoch: 7,
        sequence: 1,
        adapter_id: adapter_id(),
        auth_context_id: auth().context_id,
        tenant: "acme".to_owned(),
        region_or_project: "global".to_owned(),
        resource_kind: "repository".to_owned(),
    }
}

pub(super) fn query() -> AdapterQuery {
    AdapterQuery {
        operation: "list_repositories".to_owned(),
        authority_scope: "acme".to_owned(),
        provider_resource_type: "github.repository".to_owned(),
        filters: BTreeMap::from([(
            "visibility".to_owned(),
            SafeFieldValue::Text("all".to_owned()),
        )]),
        projection: BTreeSet::from(["name".to_owned(), "visibility".to_owned()]),
        page_size: 100,
    }
}

pub(super) fn first_request() -> AdapterPageRequest {
    AdapterPageRequest {
        protocol_version: crate::ADAPTER_PROTOCOL_VERSION,
        request_id: RequestId::new("request:00000000-0000-4000-8000-000000000002").unwrap(),
        target_id: target().target_id,
        target_identity_sha256: target().fingerprint_sha256().unwrap(),
        adapter_id: adapter_id(),
        auth_context_handle: auth().handle,
        auth_context_id: auth().context_id,
        coverage: coverage(),
        query: query(),
        page_ordinal: 0,
        cursor_handle: None,
        limits: AdapterPageLimits {
            max_records: 100,
            max_response_bytes: 1_000_000,
            max_duration_ms: 30_000,
        },
        requested_at_ms: REQUESTED_AT,
    }
}

pub(super) fn record() -> NormalizedRecord {
    NormalizedRecord::new(
        adapter_id(),
        "github".to_owned(),
        "github.repository".to_owned(),
        "acme".to_owned(),
        "repo:clark".to_owned(),
        Some("code_repository".to_owned()),
        BTreeSet::from(["private".to_owned()]),
        BTreeMap::from([
            ("name".to_owned(), SafeFieldValue::Text("clark".to_owned())),
            (
                "visibility".to_owned(),
                SafeFieldValue::Text("private".to_owned()),
            ),
        ]),
        BTreeSet::new(),
    )
    .unwrap()
}

pub(super) fn continuation_receipt() -> AdapterPageReceipt {
    AdapterPageReceipt::new(
        first_request(),
        target(),
        auth(),
        digest('6'),
        OBSERVED_AT,
        AdapterPageOutcome::Succeeded { final_page: false },
        vec![record()],
        Some(CursorHandle::new("cursor:00000000-0000-4000-8000-000000000003").unwrap()),
        RedactionSummary {
            source_records_seen: 1,
            records_emitted: 1,
            fields_omitted: 3,
            values_rejected: 0,
        },
    )
    .unwrap()
}

pub(super) fn next_request() -> AdapterPageRequest {
    let receipt = continuation_receipt();
    let mut request = receipt.request;
    request.request_id = RequestId::new("request:00000000-0000-4000-8000-000000000004").unwrap();
    request.page_ordinal = 1;
    request.cursor_handle = receipt.next_cursor_handle;
    request.requested_at_ms = CURSOR_ISSUED_AT;
    request
}
