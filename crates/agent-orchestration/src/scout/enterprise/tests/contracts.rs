use std::collections::BTreeSet;

use super::*;

#[test]
fn enterprise_ids_reject_surrounding_whitespace() {
    assert!(EnterpriseId::new(" acme ").is_err());
    assert_eq!(
        EnterpriseId::new("acme:prod").unwrap().as_str(),
        "acme:prod"
    );
}

#[test]
fn entity_identity_is_bound_to_provider_namespace_not_adapter_build() {
    let enterprise = enterprise();
    let authority = AuthorityRef::new("github", "org:acme", "repository:42").unwrap();
    let first =
        EnterpriseEntityId::derive(&enterprise, EnterpriseEntityKind::Repository, &authority)
            .unwrap();
    let second =
        EnterpriseEntityId::derive(&enterprise, EnterpriseEntityKind::Repository, &authority)
            .unwrap();
    assert_eq!(first, second);

    let other_provider = AuthorityRef::new("gitlab", "org:acme", "repository:42").unwrap();
    assert_ne!(
        first,
        EnterpriseEntityId::derive(
            &enterprise,
            EnterpriseEntityKind::Repository,
            &other_provider
        )
        .unwrap()
    );
    assert!(AuthorityRef::new("github/unsafe", "org:acme", "repository:42").is_err());
}

#[test]
fn replicated_cursors_must_be_host_issued_handles() {
    let enterprise = enterprise();
    let key = CoverageKey::new("aws", "auth", "account:a", "us-east-1", "service").unwrap();
    assert!(CoverageObservation::new(
        &enterprise,
        key.clone(),
        CoverageStatus::Truncated,
        Some("raw-provider-secret-token".into()),
        1,
        evidence('a'),
    )
    .is_err());
    assert!(CoverageObservation::new(
        &enterprise,
        key.clone(),
        CoverageStatus::Truncated,
        Some("cursor:00000000-0000-4000-8000-000000000001".into()),
        1,
        evidence('a'),
    )
    .is_ok());
    assert!(FrontierKey::new(key, Some("raw-provider-secret-token".into())).is_err());
}

#[test]
fn metadata_rejects_common_secret_canaries() {
    let enterprise = enterprise();
    for canary in [
        "GH_TOKEN=ghp_not-a-real-token",
        "-----BEGIN PRIVATE KEY-----",
        "postgres://user:password@example.invalid/db",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJzZWNyZXQtY2FuYXJ5In0.aaaaaaaaaaaaaaaa",
    ] {
        let result = GraphEntityObservation::new(
            &enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("aws", "account:prod", "service:checkout").unwrap(),
            BTreeSet::from([canary.into()]),
            evidence('a'),
        );
        assert!(result.is_err(), "accepted secret canary: {canary}");
    }
}

#[test]
fn default_internal_classification_preserves_legacy_event_encoding_and_id() {
    let event = entity("mac-a", 1, "service:checkout", "checkout");
    let encoded = serde_json::to_value(&event).unwrap();
    let observation = encoded.get("fact").expect("entity observation");
    assert!(observation.get("classification").is_none());
    assert_eq!(
        event.event_id.as_str(),
        "event:c1dc208c6329feb03d4a4a6193bae35bec9f8540527662146646855ce9fa19c4"
    );
    let decoded: EnterpriseEvent = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, event);
}

#[test]
fn do_not_store_is_rejected_before_event_construction() {
    let enterprise = enterprise();
    let mut observation = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", "service:secret").unwrap(),
        BTreeSet::from(["secret-reference".into()]),
        evidence('a'),
    )
    .unwrap();
    observation.classification = EnterpriseClassification::DoNotStore;
    let result = EnterpriseEvent::new(
        enterprise,
        provenance("mac-a", 1, 1),
        EnterpriseFact::EntityObserved(observation),
    );
    assert!(result
        .unwrap_err()
        .contains("rejected before event construction"));
}

#[test]
fn edge_classification_conservatively_joins_both_endpoints() {
    let enterprise = enterprise();
    let mut restricted = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", "service:restricted").unwrap(),
        BTreeSet::from(["restricted".into()]),
        evidence('a'),
    )
    .unwrap();
    restricted.classification = EnterpriseClassification::Restricted;
    let internal = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Repository,
        AuthorityRef::new("github", "org:acme", "repo:checkout").unwrap(),
        BTreeSet::from(["checkout".into()]),
        evidence('b'),
    )
    .unwrap();
    let edge = GraphEdgeObservation::new(
        &enterprise,
        internal.entity_id.clone(),
        restricted.entity_id.clone(),
        EnterpriseEdgeKind::SourceFor,
        None,
        evidence('c'),
    )
    .unwrap();
    let edge_id = edge.edge_id.clone();
    let batch = EnterpriseBatch::new(
        enterprise.clone(),
        [
            EnterpriseEvent::new(
                enterprise.clone(),
                provenance("mac-a", 1, 1),
                EnterpriseFact::EntityObserved(restricted),
            )
            .unwrap(),
            EnterpriseEvent::new(
                enterprise.clone(),
                provenance("mac-a", 1, 2),
                EnterpriseFact::EntityObserved(internal),
            )
            .unwrap(),
            EnterpriseEvent::new(
                enterprise.clone(),
                provenance("mac-a", 1, 3),
                EnterpriseFact::EdgeObserved(edge),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let snapshot = EnterpriseGraph::from_batches(enterprise, [batch])
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(
        snapshot.edges[&edge_id].classification,
        EnterpriseClassification::Restricted
    );
}

#[test]
fn generic_cloud_resources_preserve_provider_type_without_weakening_redaction() {
    let enterprise = enterprise();
    let mut resource = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::CloudResource,
        AuthorityRef::new(
            "aws_resource_explorer",
            "account:prod",
            "arn:aws:example:us-east-1:123456789012:resource/example",
        )
        .unwrap(),
        BTreeSet::from(["example".into()]),
        evidence('a'),
    )
    .unwrap();
    resource.provider_resource_type = Some("example:resource".into());
    assert!(EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-a", 1, 1),
        EnterpriseFact::EntityObserved(resource.clone()),
    )
    .is_ok());
    assert_eq!(
        serde_json::to_value(EnterpriseEntityKind::CloudResource).unwrap(),
        serde_json::json!("cloud_resource")
    );

    resource.provider_resource_type = Some("GH_TOKEN=ghp_not-a-real-token".into());
    assert!(EnterpriseEvent::new(
        enterprise,
        provenance("mac-a", 1, 2),
        EnterpriseFact::EntityObserved(resource),
    )
    .is_err());
}

#[test]
fn empty_coverage_rejects_nonzero_membership_counts() {
    let enterprise = enterprise();
    let key = CoverageKey::new("aws", "auth", "account:a", "us-east-1", "service").unwrap();
    assert!(CoverageObservation::new(
        &enterprise,
        key,
        CoverageStatus::Empty,
        None,
        1,
        evidence('a'),
    )
    .is_err());
}

#[test]
fn frontier_lifecycle_uses_transition_order_inside_an_epoch() {
    let enterprise = enterprise();
    let key = CoverageKey::new("aws", "auth", "account:a", "us-east-1", "service").unwrap();
    let mut pending = FrontierObservation::new(
        &enterprise,
        FrontierKey::new(key.clone(), None).unwrap(),
        FrontierState::Pending,
        BTreeSet::new(),
    )
    .unwrap();
    pending.transition_sequence = 1;
    let mut terminal = FrontierObservation::new(
        &enterprise,
        FrontierKey::new(key, None).unwrap(),
        FrontierState::Terminal {
            status: CoverageStatus::Supported,
            reason: "scan sealed".into(),
        },
        evidence('a'),
    )
    .unwrap();
    terminal.transition_sequence = 2;
    let events = [
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 1, 1),
            EnterpriseFact::FrontierObserved(pending),
        )
        .unwrap(),
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 1, 2),
            EnterpriseFact::FrontierObserved(terminal),
        )
        .unwrap(),
    ];
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(EnterpriseBatch::new(enterprise, events).unwrap())
        .unwrap();
    let snapshot = graph.snapshot().unwrap();
    let frontier = snapshot.frontier.values().next().unwrap();
    assert_eq!(frontier.transition_sequence, 2);
    assert!(frontier.is_complete());
    assert!(!snapshot
        .conflicts
        .iter()
        .any(|conflict| matches!(conflict, EnterpriseConflict::FrontierDisagreement { .. })));
}
