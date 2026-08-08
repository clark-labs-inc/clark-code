use std::collections::BTreeMap;

use scout_accumulator::{
    AccumulatorContext, AccumulatorError, Digest, MapMutationOutcome, MapStoredNode,
    PartitionedMapEditor, PartitionedMapHead, PartitionedMapUpdate, DEFAULT_PARTITION_BITS,
};

type NodeStore = BTreeMap<(u16, Digest), MapStoredNode>;

fn context(enterprise_id: &str) -> AccumulatorContext {
    AccumulatorContext::new(
        "clark.scout.enterprise-projection",
        enterprise_id,
        "current-rows",
    )
    .unwrap()
}

fn value(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

fn persist(editor: &PartitionedMapEditor, nodes: &mut NodeStore, update: &PartitionedMapUpdate) {
    for digest in &update.gc_candidates {
        nodes.remove(&(update.partition, *digest));
    }
    let partition_context = editor.head().partition_context(update.partition).unwrap();
    for node in &update.nodes {
        let digest = node.digest(&partition_context).unwrap();
        if let Some(existing) = nodes.insert((update.partition, digest), node.clone()) {
            assert_eq!(existing, *node);
        }
    }
}

fn put(
    editor: &mut PartitionedMapEditor,
    nodes: &mut NodeStore,
    object_id: &str,
    value_digest: Digest,
) -> MapMutationOutcome {
    let update = editor
        .put(object_id, value_digest, |partition, digest| {
            Ok(nodes.get(&(partition, digest)).cloned())
        })
        .unwrap();
    assert_eq!(update.next_root, editor.head().root);
    let outcome = update.outcome;
    persist(editor, nodes, &update);
    outcome
}

fn remove(
    editor: &mut PartitionedMapEditor,
    nodes: &mut NodeStore,
    object_id: &str,
) -> MapMutationOutcome {
    let update = editor
        .remove(object_id, |partition, digest| {
            Ok(nodes.get(&(partition, digest)).cloned())
        })
        .unwrap();
    assert_eq!(update.next_root, editor.head().root);
    let outcome = update.outcome;
    persist(editor, nodes, &update);
    outcome
}

fn populated(enterprise_id: &str, entries: &[(&str, u8)]) -> (PartitionedMapHead, NodeStore) {
    let head = PartitionedMapHead::empty(context(enterprise_id), DEFAULT_PARTITION_BITS).unwrap();
    let mut editor = PartitionedMapEditor::new(head).unwrap();
    let mut nodes = NodeStore::new();
    for (object_id, byte) in entries {
        assert_eq!(
            put(&mut editor, &mut nodes, object_id, value(*byte)),
            MapMutationOutcome::Inserted
        );
    }
    (editor.into_head(), nodes)
}

#[test]
fn final_map_is_independent_of_insert_order() {
    let entries = [
        ("entity:account", 1),
        ("entity:cluster", 2),
        ("entity:repository", 3),
        ("entity:runtime", 4),
        ("entity:service", 5),
        ("entity:vendor", 6),
    ];
    let expected = populated("enterprise:acme", &entries).0;
    let mut values = entries;
    let mut observed = 0;
    visit_permutations(&mut values, 0, &mut |permutation| {
        let head = populated("enterprise:acme", permutation).0;
        assert_eq!(head.root, expected.root);
        assert_eq!(head.partitions(), expected.partitions());
        observed += 1;
    });
    assert_eq!(observed, 720);
}

#[test]
fn updates_and_deletes_converge_to_the_same_final_state() {
    let initial = [
        ("entity:account", 1),
        ("entity:cluster", 2),
        ("entity:repository", 3),
        ("entity:runtime", 4),
        ("entity:service", 5),
    ];
    let (head, mut nodes) = populated("enterprise:acme", &initial);
    let mut editor = PartitionedMapEditor::new(head).unwrap();
    assert_eq!(
        put(&mut editor, &mut nodes, "entity:service", value(9)),
        MapMutationOutcome::Updated
    );
    assert_eq!(
        remove(&mut editor, &mut nodes, "entity:cluster"),
        MapMutationOutcome::Removed
    );
    assert_eq!(
        put(&mut editor, &mut nodes, "entity:repository", value(7)),
        MapMutationOutcome::Updated
    );
    assert_eq!(
        remove(&mut editor, &mut nodes, "entity:missing"),
        MapMutationOutcome::Absent
    );
    let updated = editor.into_head();

    let final_entries = [
        ("entity:runtime", 4),
        ("entity:repository", 7),
        ("entity:account", 1),
        ("entity:service", 9),
    ];
    let cold = populated("enterprise:acme", &final_entries).0;
    assert_eq!(updated.root, cold.root);
    assert_eq!(updated.partitions(), cold.partitions());

    let (head, mut nodes) = populated("enterprise:acme", &final_entries);
    let mut replay = PartitionedMapEditor::new(head).unwrap();
    assert_eq!(
        put(&mut replay, &mut nodes, "entity:cluster", value(2)),
        MapMutationOutcome::Inserted
    );
    assert_eq!(
        remove(&mut replay, &mut nodes, "entity:cluster"),
        MapMutationOutcome::Removed
    );
    assert_eq!(replay.into_head(), cold);
}

#[test]
fn independently_built_disjoint_partitions_compose_exactly() {
    let entries = (0..512_u16)
        .map(|index| (format!("entity:{index:04}"), (index % 251) as u8))
        .collect::<Vec<_>>();
    let entry_refs = entries
        .iter()
        .map(|(object_id, byte)| (object_id.as_str(), *byte))
        .collect::<Vec<_>>();
    let expected = populated("enterprise:acme", &entry_refs).0;
    let empty =
        PartitionedMapHead::empty(context("enterprise:acme"), DEFAULT_PARTITION_BITS).unwrap();
    let mut grouped = BTreeMap::<u16, Vec<(&str, u8)>>::new();
    for (object_id, byte) in &entry_refs {
        grouped
            .entry(empty.partition_for(object_id).unwrap())
            .or_default()
            .push((*object_id, *byte));
    }

    let mut partitions = BTreeMap::new();
    for (partition, entries) in grouped {
        let independently_built = populated("enterprise:acme", &entries).0;
        assert_eq!(independently_built.partitions().len(), 1);
        partitions.insert(partition, independently_built.partitions()[&partition]);
    }
    let composed = PartitionedMapHead::from_partitions(
        context("enterprise:acme"),
        DEFAULT_PARTITION_BITS,
        partitions,
    )
    .unwrap();
    assert_eq!(composed.root, expected.root);
    assert_eq!(composed.partitions(), expected.partitions());
}

#[test]
fn one_mutation_changes_only_its_partition_and_empty_partitions_disappear() {
    let (head, mut nodes) = populated(
        "enterprise:acme",
        &[
            ("entity:account", 1),
            ("entity:repository", 2),
            ("entity:service", 3),
        ],
    );
    let previous = head.clone();
    let partition = head.partition_for("entity:new").unwrap();
    let mut editor = PartitionedMapEditor::new(head).unwrap();
    assert_eq!(
        put(&mut editor, &mut nodes, "entity:new", value(4)),
        MapMutationOutcome::Inserted
    );
    for (index, partition_head) in previous.partitions() {
        if *index != partition {
            assert_eq!(editor.head().partitions().get(index), Some(partition_head));
        }
    }

    let singleton =
        PartitionedMapHead::empty(context("enterprise:singleton"), DEFAULT_PARTITION_BITS).unwrap();
    let mut singleton = PartitionedMapEditor::new(singleton).unwrap();
    let mut singleton_nodes = NodeStore::new();
    put(
        &mut singleton,
        &mut singleton_nodes,
        "entity:only",
        value(8),
    );
    assert_eq!(singleton.head().partitions().len(), 1);
    remove(&mut singleton, &mut singleton_nodes, "entity:only");
    assert_eq!(singleton.head().root.count, 0);
    assert!(singleton.head().partitions().is_empty());
}

#[test]
fn validation_rejects_tampering_versions_context_and_invalid_partitions() {
    let head = populated(
        "enterprise:acme",
        &[("entity:account", 1), ("entity:service", 2)],
    )
    .0;

    let mut tampered = head.clone();
    tampered.root.digest = Digest::from_bytes([7; 32]);
    assert_eq!(tampered.validate(), Err(AccumulatorError::RootMismatch));

    let mut unknown = head.clone();
    unknown.schema_version += 1;
    assert_eq!(
        unknown.validate(),
        Err(AccumulatorError::UnsupportedVersion)
    );

    assert!(PartitionedMapHead::from_partitions(
        context("enterprise:other"),
        DEFAULT_PARTITION_BITS,
        head.partitions().clone(),
    )
    .is_err());

    let mut value = serde_json::to_value(&head).unwrap();
    let partition_head = value["partitions"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    value["partitions"]
        .as_object_mut()
        .unwrap()
        .insert("65535".into(), partition_head);
    let invalid: PartitionedMapHead = serde_json::from_value(value).unwrap();
    assert_eq!(invalid.validate(), Err(AccumulatorError::InvalidPartition));
}

#[test]
fn root_binds_context_partitioning_and_serialized_schema() {
    let acme = populated("enterprise:acme", &[("entity:account", 1)]).0;
    let other = populated("enterprise:other", &[("entity:account", 1)]).0;
    let mut unpartitioned = PartitionedMapEditor::new(
        PartitionedMapHead::empty(context("enterprise:acme"), 0).unwrap(),
    )
    .unwrap();
    let mut nodes = NodeStore::new();
    put(&mut unpartitioned, &mut nodes, "entity:account", value(1));

    assert_ne!(acme.root.digest, other.root.digest);
    assert_ne!(acme.root.digest, unpartitioned.head().root.digest);
    assert_eq!(
        acme.root.digest.to_string(),
        "2db700b7712a98980b7771cc5251f7b69a0eb2f444cf6e9c08e09b9b246aabd2"
    );
    let bytes = serde_json::to_vec(&acme).unwrap();
    let decoded: PartitionedMapHead = serde_json::from_slice(&bytes).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, acme);

    let mut value = serde_json::to_value(&acme).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PartitionedMapHead>(value).is_err());
}

fn visit_permutations(
    values: &mut [(&str, u8)],
    index: usize,
    visitor: &mut impl FnMut(&[(&str, u8)]),
) {
    if index == values.len() {
        visitor(values);
        return;
    }
    for candidate in index..values.len() {
        values.swap(index, candidate);
        visit_permutations(values, index + 1, visitor);
        values.swap(index, candidate);
    }
}
