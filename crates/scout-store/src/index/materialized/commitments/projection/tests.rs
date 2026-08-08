use std::collections::BTreeMap;

use agent_orchestration::EnterpriseId;
use rusqlite::{params, Connection};
use scout_accumulator::{Digest, PartitionedMapHead};
use sha2::{Digest as _, Sha256};

use super::{
    build_partition, context, mutate, object_entry, partition_entries, storage, ProjectionMutation,
    PARTITION_BITS,
};
use crate::index::database::{COMMITMENT_ENTRIES_SCHEMA, INDEX_AUTH_KEY_BYTES};

const AUTH_KEY: [u8; INDEX_AUTH_KEY_BYTES] = [7; INDEX_AUTH_KEY_BYTES];

fn fixture() -> (Connection, EnterpriseId, PartitionedMapHead) {
    let connection = Connection::open_in_memory().expect("open database");
    connection
        .execute_batch(COMMITMENT_ENTRIES_SCHEMA)
        .expect("create commitment storage");
    let enterprise_id = EnterpriseId::new("projection-mutation-test").expect("enterprise id");
    let head = PartitionedMapHead::empty(context(&enterprise_id).expect("context"), PARTITION_BITS)
        .expect("empty head");
    (connection, enterprise_id, head)
}

fn entry(identity: &str, value: u64) -> (String, Digest) {
    object_entry("test", &identity, &value).expect("object entry")
}

fn put(mutation: &mut ProjectionMutation, identity: &str, value: u64) -> String {
    let (object_id, digest) = entry(identity, value);
    mutation.put("test", &identity, &value).expect("put");
    assert_eq!(entry(identity, value), (object_id.clone(), digest));
    object_id
}

#[test]
fn portable_encoding_domains_are_explicitly_v2() {
    let enterprise_id = EnterpriseId::new("projection-encoding-test").expect("enterprise id");
    assert_eq!(
        context(&enterprise_id).expect("context").namespace(),
        "materialized-v2"
    );

    let (object_id, value_digest) = entry("identity", 7);
    let v2_object = Sha256::digest(
        serde_json::to_vec(&("scout-projection-object-v2", "test", &"identity"))
            .expect("object transcript"),
    );
    let v2_value = Digest::from_bytes(
        Sha256::digest(
            serde_json::to_vec(&("scout-projection-value-v2", "test", &"identity", &7u64))
                .expect("value transcript"),
        )
        .into(),
    );
    assert_eq!(object_id, format!("test:{v2_object:x}"));
    assert_eq!(value_digest, v2_value);

    let v1_object = Sha256::digest(
        serde_json::to_vec(&("scout-projection-object-v1", "test", &"identity"))
            .expect("legacy object transcript"),
    );
    assert_ne!(object_id, format!("test:{v1_object:x}"));
}

#[test]
fn removes_present_and_absent_entries_with_exact_work() {
    let (connection, enterprise_id, head) = fixture();
    let mut initial = ProjectionMutation::default();
    put(&mut initial, "present", 1);
    let (head, _) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &initial).expect("initial put");

    let mut removal = ProjectionMutation::default();
    removal.remove("test", &"present").expect("present id");
    removal.remove("test", &"absent").expect("absent id");
    let (head, work) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &removal).expect("remove");

    assert_eq!(work.rows_deleted, 1);
    assert_eq!(work.rows_written, 0);
    assert_eq!(head.root.count, 0);
}

#[test]
fn put_wins_when_the_same_object_is_also_removed() {
    let (connection, enterprise_id, head) = fixture();
    let mut initial = ProjectionMutation::default();
    let object_id = put(&mut initial, "replace", 1);
    let (head, _) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &initial).expect("initial put");

    let mut replacement = ProjectionMutation::default();
    replacement.remove("test", &"replace").expect("remove");
    replacement.put("test", &"replace", &2u64).expect("put");
    let (head, work) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &replacement).expect("replace");

    assert_eq!((work.rows_written, work.rows_deleted), (1, 1));
    let partition = head.partition_for(&object_id).expect("partition");
    let stored =
        storage::read_projection_partition(&connection, &AUTH_KEY, partition).expect("read");
    assert_eq!(stored.get(&object_id), Some(&entry("replace", 2).1));
}

#[test]
fn disjoint_partition_is_not_read_or_rebuilt() {
    let (connection, enterprise_id, head) = fixture();
    let mut initial = ProjectionMutation::default();
    let first = put(&mut initial, "first", 1);
    let first_partition = head.partition_for(&first).expect("first partition");
    let second_identity = (0..10_000)
        .map(|index| format!("second-{index}"))
        .find(|identity| {
            head.partition_for(&entry(identity, 2).0)
                .expect("partition")
                != first_partition
        })
        .expect("disjoint object");
    let second = put(&mut initial, &second_identity, 2);
    let (head, _) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &initial).expect("initial puts");
    let second_partition = head.partition_for(&second).expect("second partition");
    let untouched = head.partitions()[&second_partition];
    connection
        .execute(
            "UPDATE commitment_entries SET mac = X'00'
             WHERE lane = 'projection-map-v2' AND partition_id = ?1",
            params![i64::from(second_partition)],
        )
        .expect("tamper disjoint partition");

    let mut change = ProjectionMutation::default();
    change.put("test", &"first", &3u64).expect("change");
    let (head, _) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &change).expect("bounded change");

    assert_eq!(head.partitions()[&second_partition], untouched);
}

#[test]
fn mutated_root_equals_a_cold_partition_rebuild() {
    let (connection, enterprise_id, head) = fixture();
    let mut initial = ProjectionMutation::default();
    put(&mut initial, "removed", 1);
    put(&mut initial, "retained", 2);
    let (head, _) =
        mutate(&connection, &AUTH_KEY, &enterprise_id, head, &initial).expect("initial puts");
    let mut change = ProjectionMutation::default();
    change.remove("test", &"removed").expect("remove");
    put(&mut change, "added", 3);
    let (head, _) = mutate(&connection, &AUTH_KEY, &enterprise_id, head, &change).expect("mutate");

    let entries = BTreeMap::from([entry("retained", 2), entry("added", 3)]);
    let partitioned = partition_entries(&head, entries).expect("partition entries");
    let partitions = partitioned
        .iter()
        .map(|(partition, entries)| {
            let context = head.partition_context(*partition).expect("context");
            (
                *partition,
                build_partition(&context, entries).expect("build"),
            )
        })
        .collect();
    let cold =
        PartitionedMapHead::from_partitions(head.context.clone(), PARTITION_BITS, partitions)
            .expect("cold head");
    assert_eq!(head, cold);
}
