use std::collections::BTreeSet;

use agent_orchestration::{
    EnterpriseClassification, EnterpriseEdgeKind, GraphEdgeObservation, MaterializedEdge,
    MaterializedEntity, QualifiedLifecycle,
};
use base64::Engine;
use rusqlite::{params, Connection};

use crate::tests::Fixture;
use crate::{
    request, EntityQuery, NeighborhoodQuery, QualifiedEntityQuery, ScoutStoreRequest,
    ScoutStoreResponse,
};

mod adversarial;

#[test]
fn qualified_history_is_half_open_restart_stable_and_cursor_authenticated() {
    let fixture = qualified_fixture();
    let first_id = current_entities(&fixture)[0].entity_id.clone();

    let at_a = qualified_entities(
        &fixture,
        QualifiedEntityQuery {
            as_of_ms: Some(150),
            max_classification: EnterpriseClassification::Internal,
            limit: 10,
            ..QualifiedEntityQuery::default()
        },
    );
    let version_a = at_a
        .entities
        .iter()
        .find(|entity| entity.entity_id == first_id)
        .expect("historical version A");
    assert!(version_a.labels.contains("version-a"));

    let at_b = qualified_entities(
        &fixture,
        QualifiedEntityQuery {
            as_of_ms: Some(200),
            max_classification: EnterpriseClassification::Internal,
            limit: 10,
            ..QualifiedEntityQuery::default()
        },
    );
    let version_b = at_b
        .entities
        .iter()
        .find(|entity| entity.entity_id == first_id)
        .expect("half-open version B");
    assert!(version_b.labels.contains("version-b"));

    let first_page_query = QualifiedEntityQuery {
        max_classification: EnterpriseClassification::Internal,
        limit: 1,
        ..QualifiedEntityQuery::default()
    };
    let first_page = qualified_entities(&fixture, first_page_query.clone());
    let restarted = qualified_entities(&fixture, first_page_query);
    assert_eq!(first_page, restarted);
    let cursor = first_page.next_cursor.expect("qualified cursor");

    let clearance_error = request(
        fixture.root.path(),
        ScoutStoreRequest::QualifiedEntities {
            enterprise_id: fixture.enterprise.clone(),
            query: QualifiedEntityQuery {
                max_classification: EnterpriseClassification::Confidential,
                cursor: Some(cursor.clone()),
                limit: 1,
                ..QualifiedEntityQuery::default()
            },
        },
    )
    .unwrap_err();
    assert!(clearance_error.contains("mismatched"), "{clearance_error}");

    let tampered = tamper_cursor_payload(&cursor);
    let tamper_error = request(
        fixture.root.path(),
        ScoutStoreRequest::QualifiedEntities {
            enterprise_id: fixture.enterprise.clone(),
            query: QualifiedEntityQuery {
                max_classification: EnterpriseClassification::Internal,
                cursor: Some(tampered),
                limit: 1,
                ..QualifiedEntityQuery::default()
            },
        },
    )
    .unwrap_err();
    assert!(
        tamper_error.contains("authentication failed"),
        "{tamper_error}"
    );
}

#[test]
fn low_clearance_neighborhood_is_unchanged_by_restricted_topology() {
    let fixture = qualified_fixture();
    let entities = current_entities(&fixture);
    let seed = entities[0].entity_id.clone();
    let query = NeighborhoodQuery {
        seed,
        depth: 3,
        limit: 10,
        as_of_ms: None,
        include_retired: false,
        max_classification: EnterpriseClassification::Internal,
    };
    let with_restricted = qualified_neighborhood(&fixture, query.clone());

    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "DELETE FROM edge_versions WHERE classification_rank > ?1",
            [i64::from(EnterpriseClassification::Internal.rank())],
        )
        .unwrap();
    connection
        .execute(
            "DELETE FROM entity_versions WHERE classification_rank > ?1",
            [i64::from(EnterpriseClassification::Internal.rank())],
        )
        .unwrap();
    drop(connection);

    let without_restricted = qualified_neighborhood(&fixture, query);
    assert_eq!(with_restricted, without_restricted);
    assert_eq!(with_restricted.entities.len(), 2);
    assert!(!with_restricted.truncated);
}

fn qualified_fixture() -> Fixture {
    let fixture = Fixture::new();
    for machine in ["machine-a", "machine-b", "machine-c"] {
        fixture.ingest(fixture.envelope(machine, 1)).unwrap();
    }
    let entities = current_entities(&fixture);
    let mut first_a = entities[0].clone();
    first_a.labels = BTreeSet::from(["version-a".into()]);
    first_a.valid_from_ms = Some(100);
    first_a.valid_to_ms = Some(200);
    first_a.qualified_pass_id = Some("pass:a".into());
    first_a.lifecycle = QualifiedLifecycle::Retired;
    let mut first_b = entities[0].clone();
    first_b.labels = BTreeSet::from(["version-b".into()]);
    first_b.valid_from_ms = Some(200);
    first_b.qualified_pass_id = Some("pass:b".into());
    let mut second = entities[1].clone();
    second.valid_from_ms = Some(100);
    second.qualified_pass_id = Some("pass:a".into());
    let mut restricted = entities[2].clone();
    restricted.valid_from_ms = Some(100);
    restricted.qualified_pass_id = Some("pass:a".into());
    restricted.classification = EnterpriseClassification::Restricted;

    let key: [u8; 32] = std::fs::read(fixture.root.path().join("private/index-auth.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    for entity in [&first_a, &first_b, &second, &restricted] {
        insert_entity_version(&connection, &key, entity);
    }

    let internal_edge = edge_version(
        &fixture,
        &entities[0],
        &entities[1],
        EnterpriseClassification::Internal,
    );
    let restricted_edge = edge_version(
        &fixture,
        &entities[1],
        &entities[2],
        EnterpriseClassification::Restricted,
    );
    insert_edge_version(&connection, &key, &internal_edge);
    insert_edge_version(&connection, &key, &restricted_edge);
    drop(connection);
    fixture
}

fn current_entities(fixture: &Fixture) -> Vec<MaterializedEntity> {
    entity_page(
        fixture,
        EntityQuery {
            limit: 10,
            ..EntityQuery::default()
        },
    )
    .entities
}

fn entity_page(fixture: &Fixture, query: EntityQuery) -> crate::EntityPage {
    let response = request(
        fixture.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: fixture.enterprise.clone(),
            query,
        },
    )
    .unwrap();
    let ScoutStoreResponse::Entities { page, .. } = response else {
        panic!("wrong current entity response");
    };
    page
}

fn qualified_entities(fixture: &Fixture, query: QualifiedEntityQuery) -> crate::EntityPage {
    let response = request(
        fixture.root.path(),
        ScoutStoreRequest::QualifiedEntities {
            enterprise_id: fixture.enterprise.clone(),
            query,
        },
    )
    .unwrap();
    let ScoutStoreResponse::Entities { page, .. } = response else {
        panic!("wrong qualified entity response");
    };
    page
}

fn qualified_neighborhood(fixture: &Fixture, query: NeighborhoodQuery) -> crate::NeighborhoodPage {
    let response = request(
        fixture.root.path(),
        ScoutStoreRequest::QualifiedNeighborhood {
            enterprise_id: fixture.enterprise.clone(),
            query,
        },
    )
    .unwrap();
    let ScoutStoreResponse::Neighborhood { page, .. } = response else {
        panic!("wrong qualified neighborhood response");
    };
    page
}

fn insert_entity_version(connection: &Connection, key: &[u8; 32], entity: &MaterializedEntity) {
    let version_key = format!("{}|{:020}", entity.entity_id, entity.valid_from_ms.unwrap());
    let kind = enum_name(&entity.kind);
    let labels = entity
        .labels
        .iter()
        .map(|label| label.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let rank = i64::from(entity.classification.rank());
    let valid_from = i64::try_from(entity.valid_from_ms.unwrap()).unwrap();
    let valid_to = entity.valid_to_ms.map(|time| i64::try_from(time).unwrap());
    let json = serde_json::to_string(entity).unwrap();
    let mac = crate::index::index_mac(
        key,
        "entity_version",
        &(
            version_key.as_str(),
            entity.entity_id.as_str(),
            kind.as_str(),
            entity.authority.provider_namespace.as_str(),
            entity.authority.authority_scope.as_str(),
            entity.critical,
            rank,
            labels.as_str(),
            valid_from,
            valid_to,
            json.as_str(),
        ),
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO entity_versions VALUES
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                version_key,
                entity.entity_id.as_str(),
                kind,
                entity.authority.provider_namespace,
                entity.authority.authority_scope,
                entity.critical,
                rank,
                labels,
                valid_from,
                valid_to,
                json,
                mac,
            ],
        )
        .unwrap();
}

fn edge_version(
    fixture: &Fixture,
    from: &MaterializedEntity,
    to: &MaterializedEntity,
    classification: EnterpriseClassification,
) -> MaterializedEdge {
    edge_version_for_kind(
        fixture,
        from,
        to,
        EnterpriseEdgeKind::ConnectedTo,
        classification,
    )
}

fn edge_version_for_kind(
    fixture: &Fixture,
    from: &MaterializedEntity,
    to: &MaterializedEntity,
    kind: EnterpriseEdgeKind,
    classification: EnterpriseClassification,
) -> MaterializedEdge {
    let observation = GraphEdgeObservation::new(
        &fixture.enterprise,
        from.entity_id.clone(),
        to.entity_id.clone(),
        kind,
        None,
        BTreeSet::from(["b".repeat(64)]),
    )
    .unwrap();
    MaterializedEdge {
        edge_id: observation.edge_id,
        from: observation.from,
        to: observation.to,
        kind: observation.kind,
        qualifier: observation.qualifier,
        classification,
        discovery_epoch_sequence: 2,
        evidence_digests: observation.evidence_digests,
        supporting_events: BTreeSet::new(),
        last_observed_at_ms: 200,
        valid_from_ms: Some(100),
        valid_to_ms: None,
        qualified_pass_id: Some("pass:a".into()),
        lifecycle: QualifiedLifecycle::Active,
    }
}

fn insert_edge_version(connection: &Connection, key: &[u8; 32], edge: &MaterializedEdge) {
    let version_key = format!("{}|{:020}", edge.edge_id, edge.valid_from_ms.unwrap());
    let kind = enum_name(&edge.kind);
    let rank = i64::from(edge.classification.rank());
    let valid_from = i64::try_from(edge.valid_from_ms.unwrap()).unwrap();
    let valid_to = edge.valid_to_ms.map(|time| i64::try_from(time).unwrap());
    let json = serde_json::to_string(edge).unwrap();
    let mac = crate::index::index_mac(
        key,
        "edge_version",
        &(
            version_key.as_str(),
            edge.edge_id.as_str(),
            edge.from.as_str(),
            edge.to.as_str(),
            kind.as_str(),
            rank,
            valid_from,
            valid_to,
            json.as_str(),
        ),
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO edge_versions VALUES
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                version_key,
                edge.edge_id.as_str(),
                edge.from.as_str(),
                edge.to.as_str(),
                kind,
                rank,
                valid_from,
                valid_to,
                json,
                mac,
            ],
        )
        .unwrap();
}

fn tamper_cursor_payload(cursor: &str) -> String {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .unwrap();
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    envelope["payload"] = serde_json::Value::String("tampered".into());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&envelope).unwrap())
}

fn enum_name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
