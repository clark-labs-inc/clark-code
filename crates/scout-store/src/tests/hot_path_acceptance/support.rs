use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, CoverageKey, CoverageObservation, CoverageStatus, EnterpriseBatch,
    EnterpriseConflict, EnterpriseEntityKind, EnterpriseFact, EnterpriseGrantScope,
    EnterpriseProvenance, EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole,
    FrontierKey, FrontierObservation, FrontierState, GraphEntityObservation,
};
use rusqlite::Connection;

use super::super::{call, status, Fixture};
use crate::{
    EdgePage, EdgeQuery, EntityPage, EntityQuery, IndexReceipt, IndexedStatus, ScoutStoreRequest,
    ScoutStoreResponse,
};

pub(super) fn signed_facts(
    fixture: &Fixture,
    machine: &str,
    epoch: u64,
    first_sequence: u64,
    facts: Vec<EnterpriseFact>,
) -> EnterpriseSignedBatch {
    let events = facts
        .into_iter()
        .enumerate()
        .map(|(offset, fact)| {
            let sequence = first_sequence + u64::try_from(offset).unwrap();
            agent_orchestration::EnterpriseEvent::new(
                fixture.enterprise.clone(),
                EnterpriseProvenance {
                    machine_id: machine.into(),
                    run_id: format!("run-{machine}-epoch-{epoch}"),
                    adapter_instance_id: "fixture-adapter".into(),
                    auth_context_id: "fixture-auth".into(),
                    discovery_epoch: format!("epoch-{epoch}"),
                    discovery_epoch_sequence: epoch,
                    source_sequence: sequence,
                    observed_at_ms: epoch * 1_000 + sequence,
                    source_fingerprint: "f".repeat(64),
                },
                fact,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let last_sequence = first_sequence + u64::try_from(events.len() - 1).unwrap();
    let batch = EnterpriseBatch::new(fixture.enterprise.clone(), events).unwrap();
    let grant = EnterpriseSignerGrant::issue(
        &fixture.manifest,
        fixture.coordinator.signer_id(),
        fixture.coordinator.public_key_hex(),
        BTreeSet::from([
            EnterpriseSignerRole::Collector,
            EnterpriseSignerRole::Coordinator,
        ]),
        EnterpriseGrantScope {
            machine_id: machine.into(),
            run_id: format!("run-{machine}-epoch-{epoch}"),
            adapter_instance_id: "fixture-adapter".into(),
            auth_context_id: "fixture-auth".into(),
            discovery_epoch: format!("epoch-{epoch}"),
            discovery_epoch_sequence: epoch,
            first_source_sequence: first_sequence,
            last_source_sequence: last_sequence,
        },
        100,
        100_000,
        &[&fixture.coordinator],
    )
    .unwrap();
    EnterpriseSignedBatch::sign(
        batch,
        &fixture.manifest,
        grant,
        10_000 + epoch,
        &fixture.coordinator,
    )
    .unwrap()
}

pub(super) fn entity(
    fixture: &Fixture,
    kind: EnterpriseEntityKind,
    native_id: &str,
) -> GraphEntityObservation {
    GraphEntityObservation::new(
        &fixture.enterprise,
        kind,
        AuthorityRef::new("fixture", "tenant:fixture", native_id).unwrap(),
        BTreeSet::from([native_id.replace(':', "-")]),
        evidence('a'),
    )
    .unwrap()
}

pub(super) fn evidence(byte: char) -> BTreeSet<String> {
    BTreeSet::from([byte.to_string().repeat(64)])
}

pub(super) fn coverage_pass(
    fixture: &Fixture,
    key: &CoverageKey,
    members: &BTreeSet<agent_orchestration::EnterpriseEntityId>,
    evidence_byte: char,
) -> Vec<EnterpriseFact> {
    let coverage = CoverageObservation::new(
        &fixture.enterprise,
        key.clone(),
        CoverageStatus::Supported,
        None,
        u64::try_from(members.len()).unwrap(),
        evidence(evidence_byte),
    )
    .unwrap();
    let mut frontier = FrontierObservation::new(
        &fixture.enterprise,
        FrontierKey::new(key.clone(), None).unwrap(),
        FrontierState::Terminal {
            status: CoverageStatus::Supported,
            reason: "authoritative fixture enumeration completed".into(),
        },
        evidence(evidence_byte),
    )
    .unwrap();
    frontier.discovered_entity_ids = members.clone();
    vec![
        EnterpriseFact::CoverageObserved(coverage),
        EnterpriseFact::FrontierObserved(frontier),
    ]
}

pub(super) fn entity_page(fixture: &Fixture) -> EntityPage {
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: fixture.enterprise.clone(),
            query: EntityQuery {
                limit: 100,
                ..EntityQuery::default()
            },
        },
    )
    .unwrap();
    let ScoutStoreResponse::Entities { page, .. } = response else {
        panic!("wrong entity page response");
    };
    page
}

pub(super) fn edge_page(fixture: &Fixture) -> EdgePage {
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Edges {
            enterprise_id: fixture.enterprise.clone(),
            query: EdgeQuery {
                limit: 100,
                ..EdgeQuery::default()
            },
        },
    )
    .unwrap();
    let ScoutStoreResponse::Edges { page, .. } = response else {
        panic!("wrong edge page response");
    };
    page
}

pub(super) fn force_cold(fixture: &Fixture) -> (IndexedStatus, IndexReceipt) {
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = 'force-cold-rebuild' WHERE key = 'projection_version'",
            [],
        )
        .unwrap();
    drop(connection);
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: fixture.enterprise.clone(),
        },
    )
    .unwrap();
    let ScoutStoreResponse::Rebuilt(receipt) = response else {
        panic!("wrong forced rebuild response");
    };
    (status(fixture).0, receipt)
}

pub(super) fn assert_roots_equal(hot: &IndexReceipt, cold: &IndexReceipt) {
    assert_eq!(hot.event_root, cold.event_root);
    assert_eq!(hot.graph_digest, cold.graph_digest);
    assert_eq!(hot.event_set_root_v1, cold.event_set_root_v1);
    assert_eq!(hot.projection_map_root_v2, cold.projection_map_root_v2);
    assert_eq!(
        hot.enterprise_snapshot_root_v2,
        cold.enterprise_snapshot_root_v2
    );
}

pub(super) fn assert_hot(response: ScoutStoreResponse) {
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong hot-path response");
    };
    assert!(!receipt.rebuilt, "append unexpectedly used a cold rebuild");
}

pub(super) fn assert_dangling_hot_equals_cold(
    fixture: &Fixture,
    expected: &BTreeSet<EnterpriseConflict>,
) {
    let hot = dangling_conflicts(fixture);
    assert_eq!(&hot, expected);
    force_cold(fixture);
    let cold = dangling_conflicts(fixture);
    assert_eq!(cold, hot);
}

fn dangling_conflicts(fixture: &Fixture) -> BTreeSet<EnterpriseConflict> {
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT materialized_json
             FROM projection_conflicts
             WHERE kind_rank = 3
             ORDER BY conflict_key",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| {
            serde_json::from_str::<EnterpriseConflict>(&row.unwrap())
                .expect("valid conflict projection")
        })
        .collect()
}
