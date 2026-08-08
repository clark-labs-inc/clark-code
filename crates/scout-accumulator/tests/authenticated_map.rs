use std::collections::BTreeMap;

use scout_accumulator::{
    plan_map_put, plan_map_remove, prove_map_persistent, verify_map_proof, AccumulatorContext,
    AccumulatorError, Digest, MapHead, MapMutation, MapMutationOutcome, MapProofStatus,
    MapStoredNode,
};

const ENTRIES: [(&str, u8); 5] = [
    ("entity:account", 1),
    ("entity:cluster", 2),
    ("entity:repository", 3),
    ("entity:runtime", 4),
    ("entity:service", 5),
];

type NodeStore = BTreeMap<Digest, MapStoredNode>;

fn context(enterprise_id: &str) -> AccumulatorContext {
    AccumulatorContext::new("clark.scout.enterprise-projection", enterprise_id, "entity").unwrap()
}

fn value(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

fn store_mutation(context: &AccumulatorContext, nodes: &mut NodeStore, mutation: &MapMutation) {
    for node in &mutation.nodes {
        let digest = node.digest(context).unwrap();
        if let Some(existing) = nodes.insert(digest, node.clone()) {
            assert_eq!(existing, *node);
        }
    }
}

fn put(
    context: &AccumulatorContext,
    head: &mut MapHead,
    nodes: &mut NodeStore,
    object_id: &str,
    digest: Digest,
) -> MapMutation {
    let mutation = plan_map_put(context, *head, object_id, digest, |node_digest| {
        Ok(nodes.get(&node_digest).cloned())
    })
    .unwrap();
    store_mutation(context, nodes, &mutation);
    *head = mutation.next;
    mutation
}

fn remove(
    context: &AccumulatorContext,
    head: &mut MapHead,
    nodes: &mut NodeStore,
    object_id: &str,
) -> MapMutation {
    let mutation = plan_map_remove(context, *head, object_id, |node_digest| {
        Ok(nodes.get(&node_digest).cloned())
    })
    .unwrap();
    store_mutation(context, nodes, &mutation);
    *head = mutation.next;
    mutation
}

fn populated(entries: &[(&str, u8)]) -> (AccumulatorContext, MapHead, NodeStore) {
    let context = context("enterprise:acme");
    let mut head = MapHead::empty(&context);
    let mut nodes = NodeStore::new();
    for (object_id, byte) in entries {
        let mutation = put(&context, &mut head, &mut nodes, object_id, value(*byte));
        assert_eq!(mutation.outcome, MapMutationOutcome::Inserted);
    }
    (context, head, nodes)
}

#[test]
fn empty_map_has_an_authenticated_absence_proof() {
    let context = context("enterprise:acme");
    let head = MapHead::empty(&context);
    let proof = prove_map_persistent(&context, head, "entity:missing", |_| Ok(None)).unwrap();
    assert_eq!(
        verify_map_proof(&head.root, &proof).unwrap(),
        MapProofStatus::Absent
    );
}

#[test]
fn insertion_permutations_converge_to_one_map_root() {
    let (_, expected, _) = populated(&ENTRIES);
    let mut entries = ENTRIES;
    let mut observed = 0;
    visit_permutations(&mut entries, 0, &mut |permutation| {
        let (_, head, _) = populated(permutation);
        assert_eq!(head.root, expected.root);
        observed += 1;
    });
    assert_eq!(observed, 120);
}

#[test]
fn updates_removals_and_reinsertion_match_cold_final_state() {
    let (context, original, mut nodes) = populated(&ENTRIES);
    let mut head = original;

    let updated = put(&context, &mut head, &mut nodes, "entity:service", value(9));
    assert_eq!(updated.outcome, MapMutationOutcome::Updated);
    assert_eq!(head.root.count, original.root.count);
    assert!(!updated.gc_candidates.is_empty());

    let unchanged = put(&context, &mut head, &mut nodes, "entity:service", value(9));
    assert_eq!(unchanged.outcome, MapMutationOutcome::Unchanged);
    assert!(unchanged.nodes.is_empty());

    let removed = remove(&context, &mut head, &mut nodes, "entity:cluster");
    assert_eq!(removed.outcome, MapMutationOutcome::Removed);
    assert_eq!(head.root.count, original.root.count - 1);
    let absent = remove(&context, &mut head, &mut nodes, "entity:missing");
    assert_eq!(absent.outcome, MapMutationOutcome::Absent);

    let final_entries = [
        ("entity:account", 1),
        ("entity:repository", 3),
        ("entity:runtime", 4),
        ("entity:service", 9),
    ];
    let (_, cold_final, _) = populated(&final_entries);
    assert_eq!(head.root, cold_final.root);

    let reinserted = put(&context, &mut head, &mut nodes, "entity:cluster", value(2));
    assert_eq!(reinserted.outcome, MapMutationOutcome::Inserted);
    let restored = put(&context, &mut head, &mut nodes, "entity:service", value(5));
    assert_eq!(restored.outcome, MapMutationOutcome::Updated);
    assert_eq!(head.root, original.root);
}

#[test]
fn persistent_proofs_bind_object_value_root_and_enterprise() {
    let (map_context, head, nodes) = populated(&ENTRIES);
    let proof = prove_map_persistent(&map_context, head, "entity:service", |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .unwrap();
    assert_eq!(
        verify_map_proof(&head.root, &proof).unwrap(),
        MapProofStatus::Present {
            value_digest: value(5)
        }
    );

    let missing = prove_map_persistent(&map_context, head, "entity:missing", |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .unwrap();
    assert_eq!(
        verify_map_proof(&head.root, &missing).unwrap(),
        MapProofStatus::Absent
    );

    let mut forged = proof.clone();
    let scout_accumulator::MapProofTerminal::Leaf { value_digest, .. } = &mut forged.terminal
    else {
        panic!("member proof must end at a leaf")
    };
    *value_digest = value(99);
    assert!(verify_map_proof(&head.root, &forged).is_err());

    let other = context("enterprise:other");
    let other_head = MapHead::empty(&other);
    assert_ne!(head.root, other_head.root);
    let mut cross_enterprise = proof;
    cross_enterprise.context = other;
    assert!(verify_map_proof(&head.root, &cross_enterprise).is_err());
}

#[test]
fn mutations_are_path_bounded_and_fail_closed_on_bad_storage() {
    let context = context("enterprise:acme");
    let mut head = MapHead::empty(&context);
    let mut nodes = NodeStore::new();
    for index in 0..4_096_u32 {
        let object_id = format!("entity:{index:08}");
        let mutation = put(
            &context,
            &mut head,
            &mut nodes,
            &object_id,
            value((index % 251) as u8),
        );
        assert!(mutation.nodes.len() <= 258);
    }

    let missing_root = head.summary.unwrap().digest;
    assert_eq!(
        plan_map_put(&context, head, "entity:new", value(7), |digest| {
            if digest == missing_root {
                Ok(None)
            } else {
                Ok(nodes.get(&digest).cloned())
            }
        })
        .unwrap_err(),
        AccumulatorError::MissingNode
    );

    let root_node = nodes.get(&missing_root).unwrap().clone();
    let mut tampered = root_node;
    match &mut tampered {
        MapStoredNode::Leaf { object_id, .. } => object_id.push_str("-forged"),
        MapStoredNode::Branch { left, .. } => left.count += 1,
    }
    assert!(
        plan_map_put(&context, head, "entity:new", value(7), |digest| {
            if digest == missing_root {
                Ok(Some(tampered.clone()))
            } else {
                Ok(nodes.get(&digest).cloned())
            }
        })
        .is_err()
    );

    let mut wrong_version = head;
    wrong_version.root.schema_version += 1;
    assert_eq!(
        plan_map_put(&context, wrong_version, "entity:new", value(7), |_| Ok(
            None
        ))
        .unwrap_err(),
        AccumulatorError::UnsupportedVersion
    );
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
