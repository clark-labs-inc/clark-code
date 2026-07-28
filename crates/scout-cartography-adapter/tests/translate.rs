use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterPageLimits, AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest,
    AdapterQuery, AuthContextDescriptor, AuthContextHandle, AuthSourceKind, CoverageBinding,
    CursorHandle, NormalizedRecord, RedactionSummary, RequestId, SafeFieldValue, TargetIdentity,
    TruncationReason,
};
use scout_cartography_adapter::{
    task_binding, translate_page, AdapterPageTaskScope, TaskBinding, ADAPTER_PAGE_TASK_KIND,
    ADAPTER_PAGE_TASK_SCOPE_VERSION,
};
use scout_ingest_protocol::cartography::{
    ClaimedTask, EvidenceObjectRef, ObservationSubject, TerminalDisposition,
};
use uuid::Uuid;

const REQUESTED_AT: u64 = 1_000;
const OBSERVED_AT: u64 = 1_100;

#[test]
fn continuation_page_becomes_entity_and_incomplete_coverage_events() {
    let receipt = receipt(
        AdapterPageOutcome::Succeeded { final_page: false },
        Some(CursorHandle::new("cursor:00000000-0000-4000-8000-000000000003").unwrap()),
    );
    let binding = binding();
    let translated = translate_page(&receipt, &evidence(), &binding).unwrap();

    assert_eq!(translated.events.len(), 2);
    assert_eq!(
        translated.completion.disposition,
        TerminalDisposition::Truncated
    );
    assert_eq!(
        translated.completion.evidence_sha256.as_deref(),
        Some(digest('b').as_str())
    );
    assert_eq!(
        translated
            .events
            .iter()
            .map(|event| event.source_sequence)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([41, 42])
    );
    let coverage = translated
        .events
        .iter()
        .find_map(|event| match &event.fact.subject {
            ObservationSubject::Coverage {
                disposition,
                complete,
                continuation_handle,
                ..
            } => Some((*disposition, *complete, continuation_handle.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        coverage,
        (
            TerminalDisposition::Truncated,
            false,
            Some("cursor:00000000-0000-4000-8000-000000000003".into())
        )
    );
    assert!(translated
        .events
        .iter()
        .all(|event| event.evidence.version_id.as_deref() == Some("version-1")));
}

#[test]
fn unrecoverable_truncation_is_an_explicit_gap_without_a_fake_cursor() {
    let receipt = receipt(
        AdapterPageOutcome::Truncated {
            reason: TruncationReason::ProviderLimit,
            continuation_available: false,
        },
        None,
    );
    let translated = translate_page(&receipt, &evidence(), &binding()).unwrap();
    assert!(translated.events.iter().any(|event| {
        matches!(
            &event.fact.subject,
            ObservationSubject::Coverage {
                disposition: TerminalDisposition::Truncated,
                complete: false,
                continuation_handle: None,
                ..
            }
        )
    }));
}

#[test]
fn claimed_task_pins_every_backend_known_discovery_choice() {
    let receipt = receipt(
        AdapterPageOutcome::Succeeded { final_page: false },
        Some(CursorHandle::new("cursor:00000000-0000-4000-8000-000000000003").unwrap()),
    );
    let task = claimed_task(&receipt);
    let binding = task_binding(&task, &receipt).unwrap();
    assert_eq!(binding.source_id, task.source_id);
    assert_eq!(binding.task_id, task.task_id);
    assert_eq!(binding.fence, task.fence);
    assert_eq!(binding.first_source_sequence, 41);

    let mut redirected_task = task;
    let mut scope: AdapterPageTaskScope =
        serde_json::from_value(redirected_task.scope.clone()).unwrap();
    scope.query.authority_scope = "another-enterprise".into();
    redirected_task.scope = serde_json::to_value(scope).unwrap();
    assert!(task_binding(&redirected_task, &receipt).is_err());
}

fn receipt(outcome: AdapterPageOutcome, cursor: Option<CursorHandle>) -> AdapterPageReceipt {
    let adapter_id = AdapterId::new("clark/github-organization@1").unwrap();
    let target = TargetIdentity::new(
        digest('1'),
        digest('2'),
        digest('3'),
        digest('4'),
        "linux".into(),
        "x86_64".into(),
    )
    .unwrap();
    let auth = AuthContextDescriptor::new(
        AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap(),
        target.target_id.clone(),
        adapter_id.clone(),
        "github".into(),
        "acme".into(),
        "principal:42".into(),
        AuthSourceKind::CliProfile,
        digest('5'),
        900,
        Some(10_000),
    )
    .unwrap();
    let request = AdapterPageRequest {
        protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
        request_id: RequestId::new("request:00000000-0000-4000-8000-000000000002").unwrap(),
        target_id: target.target_id.clone(),
        target_identity_sha256: target.fingerprint_sha256().unwrap(),
        adapter_id: adapter_id.clone(),
        auth_context_handle: auth.handle.clone(),
        auth_context_id: auth.context_id.clone(),
        coverage: CoverageBinding {
            enterprise_id: "enterprise:acme".into(),
            charter_id: "charter:topology".into(),
            discovery_epoch: 7,
            sequence: 1,
            adapter_id: adapter_id.clone(),
            auth_context_id: auth.context_id.clone(),
            tenant: "acme".into(),
            region_or_project: "global".into(),
            resource_kind: "repository".into(),
        },
        query: AdapterQuery {
            operation: "list_repositories".into(),
            authority_scope: "acme".into(),
            provider_resource_type: "github.repository".into(),
            filters: BTreeMap::new(),
            projection: BTreeSet::from(["name".into()]),
            page_size: 100,
        },
        page_ordinal: 0,
        cursor_handle: None,
        limits: AdapterPageLimits {
            max_records: 100,
            max_response_bytes: 1_000_000,
            max_duration_ms: 30_000,
        },
        requested_at_ms: REQUESTED_AT,
    };
    let record = NormalizedRecord::new(
        adapter_id,
        "github".into(),
        "github.repository".into(),
        "acme".into(),
        "repo:clark".into(),
        Some("code_repository".into()),
        BTreeSet::from(["private".into()]),
        BTreeMap::from([("name".into(), SafeFieldValue::Text("clark".into()))]),
        BTreeSet::new(),
    )
    .unwrap();
    AdapterPageReceipt::new(
        request,
        target,
        auth,
        digest('6'),
        OBSERVED_AT,
        outcome,
        vec![record],
        cursor,
        RedactionSummary {
            source_records_seen: 1,
            records_emitted: 1,
            fields_omitted: 3,
            values_rejected: 0,
        },
    )
    .unwrap()
}

fn evidence() -> EvidenceObjectRef {
    EvidenceObjectRef {
        evidence_id: format!("evidence:{}", digest('a')),
        bucket: "cartography-evidence".into(),
        key: "system-cartography/v1/receipt.json".into(),
        sha256: digest('b'),
        size_bytes: 512,
        version_id: Some("version-1".into()),
    }
}

fn binding() -> TaskBinding {
    TaskBinding {
        source_id: Uuid::new_v4(),
        task_id: Uuid::new_v4(),
        fence: 9,
        first_source_sequence: 41,
    }
}

fn claimed_task(receipt: &AdapterPageReceipt) -> ClaimedTask {
    let coverage = &receipt.request.coverage;
    let scope = AdapterPageTaskScope {
        schema_version: ADAPTER_PAGE_TASK_SCOPE_VERSION,
        first_source_sequence: 41,
        adapter_id: receipt.request.adapter_id.clone(),
        enterprise_id: coverage.enterprise_id.clone(),
        charter_id: coverage.charter_id.clone(),
        discovery_epoch: coverage.discovery_epoch,
        coverage_sequence: coverage.sequence,
        region_or_project: coverage.region_or_project.clone(),
        resource_kind: coverage.resource_kind.clone(),
        query: receipt.request.query.clone(),
        page_ordinal: receipt.request.page_ordinal,
        cursor_handle: receipt.request.cursor_handle.clone(),
        limits: receipt.request.limits,
    };
    ClaimedTask {
        task_id: Uuid::new_v4(),
        source_id: Uuid::new_v4(),
        task_kind: ADAPTER_PAGE_TASK_KIND.into(),
        scope: serde_json::to_value(scope).unwrap(),
        fence: 9,
        lease_expires_at: "2026-07-27T00:00:00Z".into(),
    }
}

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}
