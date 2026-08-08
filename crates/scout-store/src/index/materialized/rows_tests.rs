use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    AuthorityRef, EnterpriseClassification, EnterpriseEdgeId, EnterpriseEdgeKind,
    EnterpriseEntityId, EnterpriseEntityKind, MaterializedEdge, MaterializedEntity,
    QualifiedLifecycle,
};
use rusqlite::{params, Connection};

use super::{
    enum_name, index_mac, read_edges_by_ids, read_edges_incident_to_entities, read_entities_by_ids,
    upsert_edge, upsert_entity, INDEX_AUTH_KEY_BYTES, SQL_PARAMETER_CHUNK,
};

const AUTH_KEY: [u8; INDEX_AUTH_KEY_BYTES] = [0x2a; INDEX_AUTH_KEY_BYTES];

#[test]
fn edge_writer_and_reader_share_authenticated_transcript() {
    let connection = connection();
    let edge = edge(1, entity(1).entity_id, entity(2).entity_id);
    upsert_edge(&connection, &AUTH_KEY, &edge).unwrap();

    let edge_ids = BTreeSet::from([edge.edge_id.clone()]);
    assert_eq!(
        read_edges_by_ids(&connection, &AUTH_KEY, &edge_ids)
            .unwrap()
            .get(&edge.edge_id),
        Some(&edge)
    );
}

#[test]
fn targeted_readers_chunk_authenticate_and_deduplicate() {
    let mut connection = connection();
    let entity_count = SQL_PARAMETER_CHUNK + 5;
    let entities = (0..entity_count)
        .map(entity)
        .map(|entity| (entity.entity_id.clone(), entity))
        .collect::<BTreeMap<_, _>>();
    let edges = (0..entity_count - 1)
        .map(|index| edge(index, entity(index).entity_id, entity(index + 1).entity_id))
        .map(|edge| (edge.edge_id.clone(), edge))
        .collect::<BTreeMap<_, _>>();

    let transaction = connection.transaction().unwrap();
    for entity in entities.values() {
        assert!(upsert_entity(&transaction, &AUTH_KEY, entity).unwrap());
    }
    for edge in edges.values() {
        assert!(upsert_edge(&transaction, &AUTH_KEY, edge).unwrap());
    }
    transaction.commit().unwrap();

    let entity_ids = entities.keys().cloned().collect();
    let edge_ids = edges.keys().cloned().collect();
    assert_eq!(
        read_entities_by_ids(&connection, &AUTH_KEY, &entity_ids).unwrap(),
        entities
    );
    assert_eq!(
        read_edges_by_ids(&connection, &AUTH_KEY, &edge_ids).unwrap(),
        edges
    );
    assert_eq!(
        read_edges_incident_to_entities(&connection, &AUTH_KEY, &entity_ids).unwrap(),
        edges
    );
}

#[test]
fn targeted_readers_accept_empty_or_unknown_id_sets() {
    let connection = connection();
    assert!(
        read_entities_by_ids(&connection, &AUTH_KEY, &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
    assert!(read_edges_by_ids(&connection, &AUTH_KEY, &BTreeSet::new())
        .unwrap()
        .is_empty());
    assert!(
        read_edges_incident_to_entities(&connection, &AUTH_KEY, &BTreeSet::new())
            .unwrap()
            .is_empty()
    );

    let unknown_entities = BTreeSet::from([EnterpriseEntityId::new("ent:unknown").unwrap()]);
    let unknown_edges = BTreeSet::from([EnterpriseEdgeId::new("edge:unknown").unwrap()]);
    assert!(
        read_entities_by_ids(&connection, &AUTH_KEY, &unknown_entities)
            .unwrap()
            .is_empty()
    );
    assert!(read_edges_by_ids(&connection, &AUTH_KEY, &unknown_edges)
        .unwrap()
        .is_empty());
    assert!(
        read_edges_incident_to_entities(&connection, &AUTH_KEY, &unknown_entities)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn targeted_readers_reject_bad_macs_and_materialized_identity_mismatches() {
    let connection = connection();
    let first_entity = entity(1);
    let second_entity = entity(2);
    let first_edge = edge(
        1,
        first_entity.entity_id.clone(),
        second_entity.entity_id.clone(),
    );
    let second_edge = edge(
        2,
        first_entity.entity_id.clone(),
        second_entity.entity_id.clone(),
    );
    upsert_entity(&connection, &AUTH_KEY, &first_entity).unwrap();
    upsert_edge(&connection, &AUTH_KEY, &first_edge).unwrap();
    let entity_ids = BTreeSet::from([first_entity.entity_id.clone()]);
    let edge_ids = BTreeSet::from([first_edge.edge_id.clone()]);

    connection
        .execute(
            "UPDATE entities SET mac = 'bad' WHERE entity_id = ?1",
            [first_entity.entity_id.as_str()],
        )
        .unwrap();
    assert!(read_entities_by_ids(&connection, &AUTH_KEY, &entity_ids).is_err());
    upsert_entity(&connection, &AUTH_KEY, &first_entity).unwrap();

    connection
        .execute(
            "UPDATE edges SET mac = 'bad' WHERE edge_id = ?1",
            [first_edge.edge_id.as_str()],
        )
        .unwrap();
    assert!(read_edges_by_ids(&connection, &AUTH_KEY, &edge_ids).is_err());
    upsert_edge(&connection, &AUTH_KEY, &first_edge).unwrap();

    forge_entity_identity(&connection, &first_entity, &second_entity);
    assert_eq!(
        read_entities_by_ids(&connection, &AUTH_KEY, &entity_ids).unwrap_err(),
        "Scout authenticated entity row identity mismatch"
    );

    forge_edge_identity(&connection, &first_edge, &second_edge);
    assert_eq!(
        read_edges_by_ids(&connection, &AUTH_KEY, &edge_ids).unwrap_err(),
        "Scout authenticated edge row identity mismatch"
    );
}

fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE entities (
               entity_id TEXT PRIMARY KEY NOT NULL,
               kind TEXT NOT NULL,
               provider_namespace TEXT NOT NULL,
               authority_scope TEXT NOT NULL,
               critical INTEGER NOT NULL,
               classification_rank INTEGER NOT NULL,
               labels_folded TEXT NOT NULL,
               materialized_json TEXT NOT NULL,
               mac TEXT NOT NULL
             ) STRICT;
             CREATE TABLE edges (
               edge_id TEXT PRIMARY KEY NOT NULL,
               from_id TEXT NOT NULL,
               to_id TEXT NOT NULL,
               kind TEXT NOT NULL,
               classification_rank INTEGER NOT NULL,
               materialized_json TEXT NOT NULL,
               mac TEXT NOT NULL
             ) STRICT;",
        )
        .unwrap();
    connection
}

fn entity(index: usize) -> MaterializedEntity {
    MaterializedEntity {
        entity_id: EnterpriseEntityId::new(format!("ent:{index:04}")).unwrap(),
        kind: EnterpriseEntityKind::Service,
        authority: AuthorityRef::new("fixture", "tenant:test", format!("service:{index:04}"))
            .unwrap(),
        labels: BTreeSet::from([format!("service-{index:04}")]),
        environments: BTreeSet::from(["test".to_string()]),
        critical: index % 2 == 0,
        classification: EnterpriseClassification::Internal,
        discovery_epoch_sequence: 1,
        evidence_digests: BTreeSet::from(["a".repeat(64)]),
        supporting_events: BTreeSet::new(),
        last_observed_at_ms: 1,
        valid_from_ms: None,
        valid_to_ms: None,
        qualified_pass_id: None,
        lifecycle: QualifiedLifecycle::Active,
    }
}

fn edge(index: usize, from: EnterpriseEntityId, to: EnterpriseEntityId) -> MaterializedEdge {
    MaterializedEdge {
        edge_id: EnterpriseEdgeId::new(format!("edge:{index:04}")).unwrap(),
        from,
        to,
        kind: EnterpriseEdgeKind::ConnectedTo,
        qualifier: None,
        classification: EnterpriseClassification::Internal,
        discovery_epoch_sequence: 1,
        evidence_digests: BTreeSet::from(["b".repeat(64)]),
        supporting_events: BTreeSet::new(),
        last_observed_at_ms: 1,
        valid_from_ms: None,
        valid_to_ms: None,
        qualified_pass_id: None,
        lifecycle: QualifiedLifecycle::Active,
    }
}

fn forge_entity_identity(
    connection: &Connection,
    stored: &MaterializedEntity,
    replacement: &MaterializedEntity,
) {
    let kind = enum_name(&stored.kind).unwrap();
    let labels_folded = stored
        .labels
        .iter()
        .map(|label| label.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let materialized_json = serde_json::to_string(replacement).unwrap();
    let classification_rank = i64::from(stored.classification.rank());
    let mac = index_mac(
        &AUTH_KEY,
        "entity",
        &(
            stored.entity_id.as_str(),
            &kind,
            &stored.authority.provider_namespace,
            &stored.authority.authority_scope,
            stored.critical,
            classification_rank,
            &labels_folded,
            &materialized_json,
        ),
    )
    .unwrap();
    connection
        .execute(
            "UPDATE entities SET materialized_json = ?1, mac = ?2 WHERE entity_id = ?3",
            params![materialized_json, mac, stored.entity_id.as_str()],
        )
        .unwrap();
}

fn forge_edge_identity(
    connection: &Connection,
    stored: &MaterializedEdge,
    replacement: &MaterializedEdge,
) {
    let kind = enum_name(&stored.kind).unwrap();
    let materialized_json = serde_json::to_string(replacement).unwrap();
    let classification_rank = i64::from(stored.classification.rank());
    let mac = index_mac(
        &AUTH_KEY,
        "edge",
        &(
            stored.edge_id.as_str(),
            stored.from.as_str(),
            stored.to.as_str(),
            &kind,
            classification_rank,
            &materialized_json,
        ),
    )
    .unwrap();
    connection
        .execute(
            "UPDATE edges SET materialized_json = ?1, mac = ?2 WHERE edge_id = ?3",
            params![materialized_json, mac, stored.edge_id.as_str()],
        )
        .unwrap();
}
