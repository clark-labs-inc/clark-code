use std::collections::BTreeMap;

use scout_accumulator::{
    plan_partitioned_insert, AccumulatorContext, AccumulatorError, Digest, InsertOutcome,
    PartitionedAccumulatorEditor, PartitionedAccumulatorHead, StoredNode, DEFAULT_PARTITION_BITS,
};

type NodeStore = BTreeMap<(u16, Digest), StoredNode>;

fn context(enterprise_id: &str) -> AccumulatorContext {
    AccumulatorContext::new("clark.scout.enterprise-ledger", enterprise_id, "event").unwrap()
}

fn insert(
    head: &mut PartitionedAccumulatorHead,
    nodes: &mut NodeStore,
    object_id: &str,
) -> InsertOutcome {
    let mutation = plan_partitioned_insert(head.clone(), object_id, |partition, digest| {
        Ok(nodes.get(&(partition, digest)).cloned())
    })
    .unwrap();
    let partition_context = mutation.next.partition_context(mutation.partition).unwrap();
    for node in &mutation.nodes {
        let digest = node.digest(&partition_context).unwrap();
        nodes.insert((mutation.partition, digest), node.clone());
    }
    for digest in &mutation.obsolete_nodes {
        nodes.remove(&(mutation.partition, *digest));
    }
    let outcome = mutation.outcome;
    *head = mutation.next;
    outcome
}

fn populated(enterprise_id: &str, objects: &[&str]) -> (PartitionedAccumulatorHead, NodeStore) {
    let head =
        PartitionedAccumulatorHead::empty(context(enterprise_id), DEFAULT_PARTITION_BITS).unwrap();
    let mut editor = PartitionedAccumulatorEditor::new(head).unwrap();
    let mut nodes = BTreeMap::new();
    for object in objects {
        let mutation = editor
            .insert(*object, |partition, digest| {
                Ok(nodes.get(&(partition, digest)).cloned())
            })
            .unwrap();
        assert_eq!(mutation.outcome, InsertOutcome::Inserted);
        assert_eq!(mutation.next_root, editor.head().root);
        let partition_context = editor.head().partition_context(mutation.partition).unwrap();
        for node in &mutation.nodes {
            let digest = node.digest(&partition_context).unwrap();
            nodes.insert((mutation.partition, digest), node.clone());
        }
        for digest in &mutation.obsolete_nodes {
            nodes.remove(&(mutation.partition, *digest));
        }
    }
    (editor.into_head(), nodes)
}

#[test]
fn partitioned_root_is_insertion_order_independent() {
    let objects = [
        "event:alpha",
        "event:bravo",
        "event:charlie",
        "event:delta",
        "event:echo",
        "event:foxtrot",
    ];
    let permutations = [
        objects,
        [
            objects[5], objects[4], objects[3], objects[2], objects[1], objects[0],
        ],
        [
            objects[2], objects[5], objects[0], objects[4], objects[1], objects[3],
        ],
    ];
    let expected = populated("enterprise:acme", &permutations[0]).0;
    for permutation in &permutations[1..] {
        let observed = populated("enterprise:acme", permutation).0;
        assert_eq!(observed.root, expected.root);
        assert_eq!(observed.partitions(), expected.partitions());
    }
    assert_eq!(expected.root.count, objects.len() as u64);
}

#[test]
fn independently_populated_disjoint_partitions_compose_to_the_same_root() {
    let objects = (0..512)
        .map(|index| format!("event:{index:04}"))
        .collect::<Vec<_>>();
    let object_refs = objects.iter().map(String::as_str).collect::<Vec<_>>();
    let (expected, _) = populated("enterprise:acme", &object_refs);

    let empty =
        PartitionedAccumulatorHead::empty(context("enterprise:acme"), DEFAULT_PARTITION_BITS)
            .unwrap();
    let mut grouped = BTreeMap::<u16, Vec<&str>>::new();
    for object in &object_refs {
        grouped
            .entry(empty.partition_for(object).unwrap())
            .or_default()
            .push(object);
    }

    let mut independently_built = BTreeMap::new();
    for (partition, members) in grouped {
        let (head, _) = populated("enterprise:acme", &members);
        assert_eq!(head.partitions().len(), 1);
        independently_built.insert(partition, head.partitions()[&partition]);
    }
    let composed = PartitionedAccumulatorHead::from_partitions(
        context("enterprise:acme"),
        DEFAULT_PARTITION_BITS,
        independently_built,
    )
    .unwrap();

    assert_eq!(composed.root, expected.root);
    assert_eq!(composed.partitions(), expected.partitions());
}

#[test]
fn one_insert_changes_only_one_partition_and_is_idempotent() {
    let (mut head, mut nodes) = populated(
        "enterprise:acme",
        &["event:alpha", "event:bravo", "event:charlie"],
    );
    let previous = head.clone();
    let partition = head.partition_for("event:delta").unwrap();
    assert_eq!(
        insert(&mut head, &mut nodes, "event:delta"),
        InsertOutcome::Inserted
    );
    for (index, partition_head) in previous.partitions() {
        if *index != partition {
            assert_eq!(head.partitions().get(index), Some(partition_head));
        }
    }
    let root = head.root;
    assert_eq!(
        insert(&mut head, &mut nodes, "event:delta"),
        InsertOutcome::AlreadyPresent
    );
    assert_eq!(head.root, root);
}

#[test]
fn root_is_context_partitioning_and_schema_bound() {
    let acme = populated("enterprise:acme", &["event:alpha"]).0;
    let other = populated("enterprise:other", &["event:alpha"]).0;
    let unpartitioned = PartitionedAccumulatorHead::empty(context("enterprise:acme"), 0).unwrap();
    let mut nodes = BTreeMap::new();
    let mut unpartitioned = unpartitioned;
    insert(&mut unpartitioned, &mut nodes, "event:alpha");

    assert_ne!(acme.root.digest, other.root.digest);
    assert_ne!(acme.root.digest, unpartitioned.root.digest);
    assert_eq!(
        acme.root.digest.to_string(),
        "94328ff306b1f0eff4c2a81a72b29f7040e07ffb975ffc6f5fee7fdf79d245ef"
    );
    assert_eq!(acme.root.schema_version, 1);
    assert_eq!(acme.root.partition_bits, DEFAULT_PARTITION_BITS);
}

#[test]
fn validation_rejects_tampering_unknown_versions_and_bad_partitions() {
    let head = populated("enterprise:acme", &["event:alpha", "event:bravo"]).0;

    let mut tampered = head.clone();
    tampered.root.digest = Digest::from_bytes([7; 32]);
    assert_eq!(tampered.validate(), Err(AccumulatorError::RootMismatch));

    let mut unknown = head.clone();
    unknown.schema_version = 2;
    assert_eq!(
        unknown.validate(),
        Err(AccumulatorError::UnsupportedVersion)
    );

    let mut value = serde_json::to_value(&head).unwrap();
    let partitions = value["partitions"].as_object_mut().unwrap();
    let (_, partition_head) = partitions
        .iter()
        .next()
        .map(|(key, value)| (key.clone(), value.clone()))
        .unwrap();
    partitions.insert("65535".into(), partition_head);
    let invalid: PartitionedAccumulatorHead = serde_json::from_value(value).unwrap();
    assert_eq!(invalid.validate(), Err(AccumulatorError::InvalidPartition));

    let mut malformed_summary = serde_json::to_value(&head).unwrap();
    let partition = malformed_summary["partitions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    partition["summary"]["count"] = serde_json::json!(0);
    let malformed: PartitionedAccumulatorHead = serde_json::from_value(malformed_summary).unwrap();
    assert!(matches!(
        malformed.validate(),
        Err(AccumulatorError::InvalidProof(
            "non-empty subtree has a zero count"
        ))
    ));
}

#[test]
fn serialized_head_round_trips_and_rejects_unknown_fields() {
    let head = populated("enterprise:acme", &["event:alpha", "event:bravo"]).0;
    let bytes = serde_json::to_vec(&head).unwrap();
    let decoded: PartitionedAccumulatorHead = serde_json::from_slice(&bytes).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, head);

    let mut value = serde_json::to_value(&head).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PartitionedAccumulatorHead>(value).is_err());
}
