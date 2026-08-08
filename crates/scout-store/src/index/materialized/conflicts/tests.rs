use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    CoverageCellId, EnterpriseConflict, EnterpriseEdgeId, EnterpriseEntityId, EnterpriseEventId,
    EnterpriseId, EnterpriseSnapshot, FrontierTaskId,
};
use rusqlite::{params, Connection};

use super::{
    apply_affected, encode, read_affected, row_mac, stable_key, synchronize,
    update_simulation_visibility, visible_preview, ConflictScope,
};
use crate::index::database::{open_database, INDEX_AUTH_KEY_BYTES};

const AUTH_KEY: [u8; INDEX_AUTH_KEY_BYTES] = [17; INDEX_AUTH_KEY_BYTES];

fn database() -> (tempfile::TempDir, Connection) {
    let root = tempfile::tempdir().expect("temporary root");
    let connection = open_database(root.path()).expect("database");
    (root, connection)
}

fn event(value: &str) -> EnterpriseEventId {
    EnterpriseEventId::new(value).expect("event id")
}

fn events(values: &[&str]) -> BTreeSet<EnterpriseEventId> {
    values.iter().map(|value| event(value)).collect()
}

fn coverage(id: &str, event_ids: &[&str]) -> EnterpriseConflict {
    EnterpriseConflict::CoverageDisagreement {
        cell_id: CoverageCellId::new(id).expect("coverage id"),
        event_ids: events(event_ids),
    }
}

fn frontier(id: &str) -> EnterpriseConflict {
    EnterpriseConflict::FrontierDisagreement {
        task_id: FrontierTaskId::new(id).expect("frontier id"),
        event_ids: events(&["event:frontier"]),
    }
}

fn simulation(id: &str) -> EnterpriseConflict {
    EnterpriseConflict::SimulationContractDisagreement {
        runtime_id: EnterpriseEntityId::new(id).expect("runtime id"),
        event_ids: events(&["event:simulation"]),
    }
}

fn dangling(edge_id: &str, missing_id: &str) -> EnterpriseConflict {
    EnterpriseConflict::DanglingEdge {
        edge_id: EnterpriseEdgeId::new(edge_id).expect("edge id"),
        missing_entity_id: EnterpriseEntityId::new(missing_id).expect("entity id"),
    }
}

fn insert(
    connection: &Connection,
    conflicts: BTreeSet<EnterpriseConflict>,
    visibility: BTreeMap<EnterpriseEntityId, bool>,
) {
    let mutation = apply_affected(
        connection,
        &AUTH_KEY,
        &BTreeSet::new(),
        &conflicts,
        &visibility,
    )
    .expect("insert conflicts");
    assert_eq!(mutation.inserted, conflicts.len());
}

fn snapshot(conflicts: BTreeSet<EnterpriseConflict>) -> EnterpriseSnapshot {
    EnterpriseSnapshot {
        enterprise_id: EnterpriseId::new("conflict-test").expect("enterprise"),
        event_root: "event-root".into(),
        graph_digest: "graph-digest".into(),
        event_count: 0,
        retracted_event_count: 0,
        entities: BTreeMap::new(),
        edges: BTreeMap::new(),
        entity_history: BTreeMap::new(),
        edge_history: BTreeMap::new(),
        coverage: BTreeMap::new(),
        frontier: BTreeMap::new(),
        simulation_contracts: BTreeMap::new(),
        charter: None,
        discovery_passes: BTreeMap::new(),
        current_pass_id: None,
        fixed_point: false,
        control_blockers: Vec::new(),
        conflicts,
    }
}

#[test]
fn stable_identity_excludes_mutable_event_id_sets() {
    let pairs = [
        (
            EnterpriseConflict::SourceEquivocation {
                source_position: "source:one".into(),
                event_ids: events(&["event:one"]),
            },
            EnterpriseConflict::SourceEquivocation {
                source_position: "source:one".into(),
                event_ids: events(&["event:two"]),
            },
        ),
        (
            coverage("coverage:one", &["event:one"]),
            coverage("coverage:one", &["event:two"]),
        ),
        (
            EnterpriseConflict::FrontierDisagreement {
                task_id: FrontierTaskId::new("frontier:one").expect("frontier"),
                event_ids: events(&["event:one"]),
            },
            EnterpriseConflict::FrontierDisagreement {
                task_id: FrontierTaskId::new("frontier:one").expect("frontier"),
                event_ids: events(&["event:two"]),
            },
        ),
        (
            EnterpriseConflict::SimulationContractDisagreement {
                runtime_id: EnterpriseEntityId::new("ent:one").expect("runtime"),
                event_ids: events(&["event:one"]),
            },
            EnterpriseConflict::SimulationContractDisagreement {
                runtime_id: EnterpriseEntityId::new("ent:one").expect("runtime"),
                event_ids: events(&["event:two"]),
            },
        ),
        (
            EnterpriseConflict::CharterDisagreement {
                event_ids: events(&["event:one"]),
            },
            EnterpriseConflict::CharterDisagreement {
                event_ids: events(&["event:two"]),
            },
        ),
        (
            EnterpriseConflict::DiscoveryPassFork {
                discovery_epoch_sequence: 7,
                pass_ids: BTreeSet::from(["pass:one".into()]),
            },
            EnterpriseConflict::DiscoveryPassFork {
                discovery_epoch_sequence: 7,
                pass_ids: BTreeSet::from(["pass:two".into()]),
            },
        ),
    ];
    for (first, second) in pairs {
        assert_ne!(first, second);
        assert_eq!(
            stable_key(&first).expect("first key"),
            stable_key(&second).expect("second key")
        );
    }
}

#[test]
fn cold_synchronize_inserts_updates_and_deletes_stable_rows() {
    let (_root, connection) = database();
    let first = coverage("coverage:stable", &["event:one"]);
    let second = coverage("coverage:stable", &["event:two"]);
    let inserted = synchronize(&connection, &AUTH_KEY, &snapshot(BTreeSet::from([first])))
        .expect("initial synchronization");
    assert_eq!(
        (inserted.inserted, inserted.updated, inserted.deleted),
        (1, 0, 0)
    );
    let updated = synchronize(&connection, &AUTH_KEY, &snapshot(BTreeSet::from([second])))
        .expect("replacement synchronization");
    assert_eq!(
        (updated.inserted, updated.updated, updated.deleted),
        (0, 1, 0)
    );
    assert_eq!(updated.rows_read, 1);
    let deleted = synchronize(&connection, &AUTH_KEY, &snapshot(BTreeSet::new()))
        .expect("empty synchronization");
    assert_eq!(
        (deleted.inserted, deleted.updated, deleted.deleted),
        (0, 0, 1)
    );
}

#[test]
fn affected_reads_return_only_targeted_rows() {
    let (_root, connection) = database();
    let target_coverage = coverage("coverage:target", &["event:coverage"]);
    let target_frontier = frontier("frontier:target");
    let target_simulation = simulation("ent:target");
    let target_dangling = dangling("edge:target", "ent:missing");
    let conflicts = BTreeSet::from([
        target_coverage.clone(),
        coverage("coverage:other", &["event:other"]),
        target_frontier.clone(),
        frontier("frontier:other"),
        target_simulation.clone(),
        simulation("ent:other"),
        target_dangling.clone(),
        dangling("edge:other", "ent:missing-other"),
        EnterpriseConflict::CharterDisagreement {
            event_ids: events(&["event:charter"]),
        },
    ]);
    insert(
        &connection,
        conflicts,
        BTreeMap::from([
            (EnterpriseEntityId::new("ent:target").expect("target"), true),
            (EnterpriseEntityId::new("ent:other").expect("other"), true),
        ]),
    );
    let scope = ConflictScope {
        coverage: BTreeSet::from([CoverageCellId::new("coverage:target").expect("coverage")]),
        simulation: BTreeSet::from([EnterpriseEntityId::new("ent:target").expect("simulation")]),
        frontier: BTreeSet::from([FrontierTaskId::new("frontier:target").expect("frontier")]),
        dangling_edges: BTreeSet::from([EnterpriseEdgeId::new("edge:target").expect("edge")]),
    };
    let read = read_affected(&connection, &AUTH_KEY, &scope).expect("affected conflicts");
    assert_eq!(read.rows_read, 4);
    assert_eq!(
        read.conflicts,
        BTreeSet::from([
            target_coverage,
            target_frontier,
            target_simulation,
            target_dangling,
        ])
    );
}

#[test]
fn authenticated_rows_reject_mac_and_identity_tampering() {
    let (_root, connection) = database();
    let target = coverage("coverage:target", &["event:one"]);
    insert(
        &connection,
        BTreeSet::from([target.clone()]),
        BTreeMap::new(),
    );
    connection
        .execute(
            "UPDATE projection_conflicts SET materialized_json = 'tampered'",
            [],
        )
        .expect("tamper");
    let scope = ConflictScope {
        coverage: BTreeSet::from([CoverageCellId::new("coverage:target").expect("coverage")]),
        ..ConflictScope::default()
    };
    assert!(read_affected(&connection, &AUTH_KEY, &scope)
        .expect_err("MAC tamper must fail")
        .contains("authentication failed"));

    connection
        .execute("DELETE FROM projection_conflicts", [])
        .expect("clear");
    let mut row = encode(&target, true, &AUTH_KEY).expect("encoded row");
    row.5 =
        serde_json::to_string(&coverage("coverage:other", &["event:two"])).expect("conflict json");
    row.6 = row_mac(&AUTH_KEY, &row).expect("valid MAC over mismatched identity");
    connection
        .execute(
            "INSERT INTO projection_conflicts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.0, row.1, row.2, row.3, row.4, row.5, row.6],
        )
        .expect("identity-tampered row");
    assert!(read_affected(&connection, &AUTH_KEY, &scope)
        .expect_err("identity tamper must fail")
        .contains("identity mismatch"));
}

#[test]
fn visibility_is_internal_safe_and_point_updatable() {
    let (_root, connection) = database();
    let visible = coverage("coverage:visible", &["event:coverage"]);
    let hidden = dangling("edge:hidden", "ent:missing");
    let runtime = EnterpriseEntityId::new("ent:runtime").expect("runtime");
    let simulation = simulation(runtime.as_str());
    insert(
        &connection,
        BTreeSet::from([visible.clone(), hidden, simulation.clone()]),
        BTreeMap::from([(runtime.clone(), false)]),
    );
    assert_eq!(
        visible_preview(&connection, &AUTH_KEY, 64)
            .expect("preview")
            .conflicts,
        vec![visible.clone()]
    );
    let mutation =
        update_simulation_visibility(&connection, &AUTH_KEY, &BTreeMap::from([(runtime, true)]))
            .expect("visibility update");
    assert_eq!(mutation.updated, 1);
    assert_eq!(mutation.visible_delta, 1);
    assert_eq!(mutation.rows_read, 1);
    assert_eq!(mutation.commitment_puts().count(), 0);
    assert_eq!(
        visible_preview(&connection, &AUTH_KEY, 64)
            .expect("preview")
            .conflicts,
        vec![visible, simulation]
    );
}

#[test]
fn affected_apply_updates_and_deletes_one_stable_row() {
    let (_root, connection) = database();
    let old = coverage("coverage:stable", &["event:one"]);
    let new = coverage("coverage:stable", &["event:one", "event:two"]);
    insert(&connection, BTreeSet::from([old.clone()]), BTreeMap::new());
    let changed = apply_affected(
        &connection,
        &AUTH_KEY,
        &BTreeSet::from([old]),
        &BTreeSet::from([new.clone()]),
        &BTreeMap::new(),
    )
    .expect("update");
    assert_eq!(
        (changed.inserted, changed.updated, changed.deleted),
        (0, 1, 0)
    );
    assert_eq!(changed.rows_read, 2);
    assert_eq!(changed.commitment_puts().count(), 1);
    assert_eq!(changed.commitment_removals().count(), 0);

    let removed = apply_affected(
        &connection,
        &AUTH_KEY,
        &BTreeSet::from([new]),
        &BTreeSet::new(),
        &BTreeMap::new(),
    )
    .expect("delete");
    assert_eq!(
        (removed.inserted, removed.updated, removed.deleted),
        (0, 0, 1)
    );
    assert_eq!(removed.rows_read, 2);
    assert_eq!(removed.commitment_puts().count(), 0);
    assert_eq!(removed.commitment_removals().count(), 1);
}

#[test]
fn visible_preview_is_exactly_rust_ordered_and_bounded_to_64() {
    let (_root, connection) = database();
    let conflicts = (0..80)
        .map(|index| {
            coverage(
                &format!("coverage:{index:03}"),
                &[&format!("event:{index:03}")],
            )
        })
        .collect::<BTreeSet<_>>();
    insert(&connection, conflicts.clone(), BTreeMap::new());
    let expected = conflicts.into_iter().take(64).collect::<Vec<_>>();
    let preview = visible_preview(&connection, &AUTH_KEY, 64).expect("preview");
    assert_eq!(preview.rows_read, 64);
    assert_eq!(preview.conflicts, expected);
    assert!(visible_preview(&connection, &AUTH_KEY, 65).is_err());
}
