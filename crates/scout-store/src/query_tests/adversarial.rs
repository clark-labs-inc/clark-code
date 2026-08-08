use agent_orchestration::{
    EnterpriseClassification, EnterpriseEdgeKind, MaterializedEdge, MaterializedEntity,
};
use base64::Engine;
use rusqlite::{params, Connection};

use super::{
    current_entities, edge_version_for_kind, insert_edge_version, qualified_entities,
    qualified_fixture, qualified_neighborhood, Fixture,
};
use crate::{
    request, EdgeQuery, EntityQuery, NeighborhoodQuery, QualifiedEdgeQuery, QualifiedEntityQuery,
    ScoutStoreRequest, ScoutStoreResponse,
};

mod fixture;

use fixture::ForeignFixture;

#[test]
fn foreign_enterprise_rows_and_cursors_are_rejected() {
    let source = ForeignFixture::new("enterprise-source", 0x31);
    let target = ForeignFixture::new("enterprise-target", 0x32);
    for machine in ["machine-a", "machine-b"] {
        source.ingest(machine);
        target.ingest(machine);
    }

    let source_page = source.entity_page(EntityQuery {
        limit: 1,
        ..EntityQuery::default()
    });
    let foreign_cursor = source_page.next_cursor.expect("source cursor");
    let cursor_error = request(
        target.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: target.enterprise.clone(),
            query: EntityQuery {
                cursor: Some(foreign_cursor),
                limit: 1,
                ..EntityQuery::default()
            },
        },
    )
    .unwrap_err();
    assert!(
        cursor_error.contains("authentication failed"),
        "{cursor_error}"
    );

    let source_connection = Connection::open(source.index_path()).unwrap();
    let transplanted = source_connection
        .query_row(
            "SELECT entity_id, kind, provider_namespace, authority_scope, critical,
                    classification_rank, labels_folded, materialized_json, mac
             FROM entities LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .unwrap();
    drop(source_connection);
    let target_connection = Connection::open(target.index_path()).unwrap();
    target_connection
        .execute(
            "INSERT INTO entities VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                transplanted.0,
                transplanted.1,
                transplanted.2,
                transplanted.3,
                transplanted.4,
                transplanted.5,
                transplanted.6,
                transplanted.7,
                transplanted.8,
            ],
        )
        .unwrap();
    drop(target_connection);

    let row_error = request(
        target.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: target.enterprise.clone(),
            query: EntityQuery {
                limit: 100,
                ..EntityQuery::default()
            },
        },
    )
    .unwrap_err();
    assert!(row_error.contains("authentication failed"), "{row_error}");
}

#[test]
fn every_single_byte_cursor_mutation_is_rejected() {
    let fixture = qualified_fixture();
    let cursor = qualified_entities(
        &fixture,
        QualifiedEntityQuery {
            max_classification: EnterpriseClassification::Internal,
            limit: 1,
            ..QualifiedEntityQuery::default()
        },
    )
    .next_cursor
    .expect("qualified cursor");
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .unwrap();
    assert!(encoded.len() > 32);

    for index in 0..encoded.len() {
        let mut mutated = encoded.clone();
        mutated[index] ^= 0x01;
        let cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mutated);
        let result = request(
            fixture.root.path(),
            ScoutStoreRequest::QualifiedEntities {
                enterprise_id: fixture.enterprise.clone(),
                query: QualifiedEntityQuery {
                    max_classification: EnterpriseClassification::Internal,
                    cursor: Some(cursor),
                    limit: 1,
                    ..QualifiedEntityQuery::default()
                },
            },
        );
        assert!(
            result.is_err(),
            "mutating decoded cursor byte {index} was accepted"
        );
    }
}

#[test]
fn low_clearance_pages_are_byte_identical_without_restricted_topology() {
    let fixture = classification_fixture();
    let entities = current_entities(&fixture);
    let seed = entities[0].entity_id.clone();
    let with_restricted = low_clearance_surfaces(&fixture, seed.clone());

    assert_eq!(with_restricted.current_entity_pages.len(), 2);
    assert_eq!(with_restricted.current_edge_pages.len(), 2);
    assert_eq!(with_restricted.qualified_entity_pages.len(), 3);
    assert_eq!(with_restricted.qualified_edge_pages.len(), 2);

    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    for table in ["edges", "entities", "edge_versions", "entity_versions"] {
        connection
            .execute(
                &format!("DELETE FROM {table} WHERE classification_rank > ?1"),
                [i64::from(EnterpriseClassification::Internal.rank())],
            )
            .unwrap();
    }
    drop(connection);

    let without_restricted = low_clearance_surfaces(&fixture, seed);
    assert_eq!(with_restricted, without_restricted);
}

#[derive(Debug, PartialEq, Eq)]
struct LowClearanceSurfaces {
    current_entity_pages: Vec<Vec<u8>>,
    current_edge_pages: Vec<Vec<u8>>,
    current_neighborhood: Vec<u8>,
    qualified_entity_pages: Vec<Vec<u8>>,
    qualified_edge_pages: Vec<Vec<u8>>,
    qualified_neighborhood: Vec<u8>,
}

fn classification_fixture() -> Fixture {
    let fixture = qualified_fixture();
    let entities = current_entities(&fixture);
    let key: [u8; 32] = std::fs::read(fixture.root.path().join("private/index-auth.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();

    let mut restricted_entity = entities[2].clone();
    restricted_entity.classification = EnterpriseClassification::Restricted;
    insert_current_entity(&connection, &key, &restricted_entity);

    let connected = edge_version_for_kind(
        &fixture,
        &entities[0],
        &entities[1],
        EnterpriseEdgeKind::ConnectedTo,
        EnterpriseClassification::Internal,
    );
    let depends_on = edge_version_for_kind(
        &fixture,
        &entities[0],
        &entities[1],
        EnterpriseEdgeKind::DependsOn,
        EnterpriseClassification::Internal,
    );
    let restricted_edge = edge_version_for_kind(
        &fixture,
        &entities[1],
        &entities[2],
        EnterpriseEdgeKind::ConnectedTo,
        EnterpriseClassification::Restricted,
    );
    for edge in [&connected, &depends_on, &restricted_edge] {
        insert_current_edge(&connection, &key, edge);
    }
    insert_edge_version(&connection, &key, &depends_on);
    drop(connection);
    fixture
}

fn low_clearance_surfaces(
    fixture: &Fixture,
    seed: agent_orchestration::EnterpriseEntityId,
) -> LowClearanceSurfaces {
    let current_neighborhood = request(
        fixture.root.path(),
        ScoutStoreRequest::Neighborhood {
            enterprise_id: fixture.enterprise.clone(),
            seed: seed.clone(),
            depth: 8,
            limit: 10,
        },
    )
    .unwrap();
    let ScoutStoreResponse::Neighborhood {
        page: current_neighborhood,
        ..
    } = current_neighborhood
    else {
        panic!("wrong current neighborhood response");
    };
    let qualified_neighborhood = qualified_neighborhood(
        fixture,
        NeighborhoodQuery {
            seed,
            depth: 8,
            limit: 10,
            as_of_ms: None,
            include_retired: false,
            max_classification: EnterpriseClassification::Internal,
        },
    );

    LowClearanceSurfaces {
        current_entity_pages: current_entity_page_bytes(fixture),
        current_edge_pages: current_edge_page_bytes(fixture),
        current_neighborhood: serde_json::to_vec(&current_neighborhood).unwrap(),
        qualified_entity_pages: qualified_entity_page_bytes(fixture),
        qualified_edge_pages: qualified_edge_page_bytes(fixture),
        qualified_neighborhood: serde_json::to_vec(&qualified_neighborhood).unwrap(),
    }
}

fn current_entity_page_bytes(fixture: &Fixture) -> Vec<Vec<u8>> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let page = super::entity_page(
            fixture,
            EntityQuery {
                cursor,
                limit: 1,
                ..EntityQuery::default()
            },
        );
        cursor = page.next_cursor.clone();
        pages.push(serde_json::to_vec(&page).unwrap());
        if cursor.is_none() {
            return pages;
        }
    }
}

fn current_edge_page_bytes(fixture: &Fixture) -> Vec<Vec<u8>> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let response = request(
            fixture.root.path(),
            ScoutStoreRequest::Edges {
                enterprise_id: fixture.enterprise.clone(),
                query: EdgeQuery {
                    cursor,
                    limit: 1,
                    ..EdgeQuery::default()
                },
            },
        )
        .unwrap();
        let ScoutStoreResponse::Edges { page, .. } = response else {
            panic!("wrong current edge response");
        };
        cursor = page.next_cursor.clone();
        pages.push(serde_json::to_vec(&page).unwrap());
        if cursor.is_none() {
            return pages;
        }
    }
}

fn qualified_entity_page_bytes(fixture: &Fixture) -> Vec<Vec<u8>> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let page = qualified_entities(
            fixture,
            QualifiedEntityQuery {
                include_retired: true,
                max_classification: EnterpriseClassification::Internal,
                cursor,
                limit: 1,
                ..QualifiedEntityQuery::default()
            },
        );
        cursor = page.next_cursor.clone();
        pages.push(serde_json::to_vec(&page).unwrap());
        if cursor.is_none() {
            return pages;
        }
    }
}

fn qualified_edge_page_bytes(fixture: &Fixture) -> Vec<Vec<u8>> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let response = request(
            fixture.root.path(),
            ScoutStoreRequest::QualifiedEdges {
                enterprise_id: fixture.enterprise.clone(),
                query: QualifiedEdgeQuery {
                    include_retired: true,
                    max_classification: EnterpriseClassification::Internal,
                    cursor,
                    limit: 1,
                    ..QualifiedEdgeQuery::default()
                },
            },
        )
        .unwrap();
        let ScoutStoreResponse::Edges { page, .. } = response else {
            panic!("wrong qualified edge response");
        };
        cursor = page.next_cursor.clone();
        pages.push(serde_json::to_vec(&page).unwrap());
        if cursor.is_none() {
            return pages;
        }
    }
}

fn insert_current_entity(connection: &Connection, key: &[u8; 32], entity: &MaterializedEntity) {
    let kind = super::enum_name(&entity.kind);
    let labels = entity
        .labels
        .iter()
        .map(|label| label.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let rank = i64::from(entity.classification.rank());
    let json = serde_json::to_string(entity).unwrap();
    let mac = crate::index::index_mac(
        key,
        "entity",
        &(
            entity.entity_id.as_str(),
            kind.as_str(),
            entity.authority.provider_namespace.as_str(),
            entity.authority.authority_scope.as_str(),
            entity.critical,
            rank,
            labels.as_str(),
            json.as_str(),
        ),
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO entities VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(entity_id) DO UPDATE SET
               kind=excluded.kind, provider_namespace=excluded.provider_namespace,
               authority_scope=excluded.authority_scope, critical=excluded.critical,
               classification_rank=excluded.classification_rank,
               labels_folded=excluded.labels_folded,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                entity.entity_id.as_str(),
                kind,
                entity.authority.provider_namespace,
                entity.authority.authority_scope,
                entity.critical,
                rank,
                labels,
                json,
                mac,
            ],
        )
        .unwrap();
}

fn insert_current_edge(connection: &Connection, key: &[u8; 32], edge: &MaterializedEdge) {
    let kind = super::enum_name(&edge.kind);
    let rank = i64::from(edge.classification.rank());
    let json = serde_json::to_string(edge).unwrap();
    let mac = crate::index::index_mac(
        key,
        "edge",
        &(
            edge.edge_id.as_str(),
            edge.from.as_str(),
            edge.to.as_str(),
            kind.as_str(),
            rank,
            json.as_str(),
        ),
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO edges VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(edge_id) DO UPDATE SET
               from_id=excluded.from_id, to_id=excluded.to_id, kind=excluded.kind,
               classification_rank=excluded.classification_rank,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                edge.edge_id.as_str(),
                edge.from.as_str(),
                edge.to.as_str(),
                kind,
                rank,
                json,
                mac,
            ],
        )
        .unwrap();
}
