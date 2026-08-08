use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};

use super::Fixture;
use crate::{IndexReceipt, ScoutStoreResponse};

#[test]
fn disjoint_partition_commitment_tamper_cannot_receive_unqualified_roots() {
    let fixture = Fixture::new();
    let probe = Fixture::new();
    for index in 0..8 {
        let machine = format!("seed-{index}");
        fixture.ingest(fixture.envelope(&machine, 1)).unwrap();
        probe.ingest(probe.envelope(&machine, 1)).unwrap();
    }

    let fixture_rows = commitment_rows(&fixture);
    let probe_before = commitment_rows(&probe);
    assert_same_commitment_identities(&fixture_rows, &probe_before);

    let append = probe.envelope("unrelated-append", 1);
    let response = probe.ingest(append.clone()).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong probe append response");
    };
    assert!(
        !receipt.rebuilt,
        "probe append did not exercise the hot path"
    );
    let probe_after = commitment_rows(&probe);
    let touched_partitions = changed_partitions(&probe_before, &probe_after);
    assert!(
        touched_partitions
            .iter()
            .any(|(lane, _)| lane == "event-set-v1"),
        "probe did not identify the appended event partition"
    );
    assert!(
        touched_partitions
            .iter()
            .any(|(lane, _)| lane == "projection-map-v2"),
        "probe did not identify appended projection partitions"
    );

    let tampered = fixture_rows
        .iter()
        .find(|row| {
            row.lane == "projection-map-v2"
                && !touched_partitions.contains(&(row.lane.clone(), row.partition))
        })
        .or_else(|| {
            fixture_rows
                .iter()
                .find(|row| !touched_partitions.contains(&(row.lane.clone(), row.partition)))
        })
        .expect("seed fixture must contain a commitment partition disjoint from the append");
    let counts_before = commitment_lane_counts(&fixture);
    let connection = open_index(&fixture);
    assert_eq!(
        connection
            .execute(
                "UPDATE commitment_entries SET mac = X'00'
                 WHERE lane = ?1 AND partition_id = ?2 AND object_id = ?3",
                params![tampered.lane, tampered.partition, tampered.object_id],
            )
            .unwrap(),
        1
    );
    drop(connection);
    assert_eq!(
        commitment_lane_counts(&fixture),
        counts_before,
        "the adversarial mutation must preserve lane counts"
    );

    match fixture.ingest(append) {
        Err(_) => {}
        Ok(ScoutStoreResponse::Ingested { receipt, .. }) if receipt.rebuilt => {}
        Ok(ScoutStoreResponse::Ingested { receipt, .. }) => {
            assert_no_unqualified_supplemental_roots(&receipt, tampered, &touched_partitions);
        }
        Ok(response) => panic!("wrong append response after commitment tamper: {response:?}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommitmentRow {
    lane: String,
    partition: i64,
    object_id: String,
    value_digest: Vec<u8>,
    mac: Vec<u8>,
}

type CommitmentPartition = (String, i64);
type CommitmentPartitionEntry = (String, Vec<u8>, Vec<u8>);
type CommitmentRowsByPartition = BTreeMap<CommitmentPartition, Vec<CommitmentPartitionEntry>>;

fn commitment_rows(fixture: &Fixture) -> Vec<CommitmentRow> {
    let connection = open_index(fixture);
    let mut statement = connection
        .prepare(
            "SELECT lane, partition_id, object_id, value_digest, mac
             FROM commitment_entries
             ORDER BY lane, partition_id, object_id",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok(CommitmentRow {
                lane: row.get(0)?,
                partition: row.get(1)?,
                object_id: row.get(2)?,
                value_digest: row.get(3)?,
                mac: row.get(4)?,
            })
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn changed_partitions(
    before: &[CommitmentRow],
    after: &[CommitmentRow],
) -> BTreeSet<CommitmentPartition> {
    let before = rows_by_partition(before);
    let after = rows_by_partition(after);
    before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

fn rows_by_partition(rows: &[CommitmentRow]) -> CommitmentRowsByPartition {
    let mut partitions = BTreeMap::new();
    for row in rows {
        partitions
            .entry((row.lane.clone(), row.partition))
            .or_insert_with(Vec::new)
            .push((
                row.object_id.clone(),
                row.value_digest.clone(),
                row.mac.clone(),
            ));
    }
    partitions
}

fn assert_same_commitment_identities(left: &[CommitmentRow], right: &[CommitmentRow]) {
    let identities = |rows: &[CommitmentRow]| {
        rows.iter()
            .map(|row| {
                (
                    row.lane.clone(),
                    row.partition,
                    row.object_id.clone(),
                    row.value_digest.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(identities(left), identities(right));
}

fn commitment_lane_counts(fixture: &Fixture) -> BTreeMap<String, i64> {
    let connection = open_index(fixture);
    let mut statement = connection
        .prepare(
            "SELECT lane, COUNT(*) FROM commitment_entries
             GROUP BY lane ORDER BY lane",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn assert_no_unqualified_supplemental_roots(
    receipt: &IndexReceipt,
    tampered: &CommitmentRow,
    touched_partitions: &BTreeSet<CommitmentPartition>,
) {
    assert!(
        receipt.event_set_root_v1.is_none()
            && receipt.projection_map_root_v2.is_none()
            && receipt.enterprise_snapshot_root_v2.is_none(),
        "hot append issued supplemental roots without authenticating disjoint tampered entry \
         lane={} partition={} object_id={}; append touched {:?}",
        tampered.lane,
        tampered.partition,
        tampered.object_id,
        touched_partitions
    );
}

fn open_index(fixture: &Fixture) -> Connection {
    Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap()
}
