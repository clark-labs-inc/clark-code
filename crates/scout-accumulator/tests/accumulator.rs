use std::collections::BTreeMap;

use scout_accumulator::{
    plan_insert, prove_persistent, verify_proof, Accumulator, AccumulatorContext, AccumulatorError,
    AccumulatorHead, Digest, Direction, InsertOutcome, ProofStatus, ProofTerminal, StoredNode,
};

const OBJECTS: [&str; 6] = [
    "batch:alpha",
    "batch:bravo",
    "batch:charlie",
    "batch:delta",
    "batch:echo",
    "batch:foxtrot",
];

fn context() -> AccumulatorContext {
    AccumulatorContext::new("scout-ledger", "enterprise:acme", "batch").unwrap()
}

fn populated(objects: &[&str]) -> Accumulator {
    let mut accumulator = Accumulator::new(context());
    for object in objects {
        assert_eq!(
            accumulator.insert(*object).unwrap(),
            InsertOutcome::Inserted
        );
    }
    accumulator
}

#[test]
fn empty_set_has_an_authenticated_nonmembership_proof() {
    let accumulator = Accumulator::new(context());
    let root = accumulator.root();
    assert_eq!(root.count, 0);

    let proof = accumulator.prove("batch:missing");
    assert_eq!(proof.terminal, ProofTerminal::Empty);
    assert!(proof.steps.is_empty());
    assert_eq!(verify_proof(&root, &proof).unwrap(), ProofStatus::NonMember);
}

#[test]
fn insert_is_idempotent_and_membership_is_authenticated() {
    let mut accumulator = Accumulator::new(context());
    assert_eq!(
        accumulator.insert("batch:alpha").unwrap(),
        InsertOutcome::Inserted
    );
    let root = accumulator.root();
    assert_eq!(
        accumulator.insert("batch:alpha").unwrap(),
        InsertOutcome::AlreadyPresent
    );
    assert_eq!(accumulator.root(), root);
    assert_eq!(accumulator.count(), 1);
    assert!(accumulator.contains("batch:alpha"));
    assert!(!accumulator.contains("batch:missing"));

    let proof = accumulator.prove("batch:alpha");
    assert_eq!(verify_proof(&root, &proof).unwrap(), ProofStatus::Member);
}

#[test]
fn all_insertion_permutations_converge_to_one_root() {
    let expected = populated(&OBJECTS).root();
    let mut values = OBJECTS;
    let mut observed = 0_usize;
    visit_permutations(&mut values, 0, &mut |permutation| {
        let accumulator = populated(permutation);
        assert_eq!(accumulator.root(), expected);
        assert_eq!(accumulator.count(), OBJECTS.len() as u64);
        for object in OBJECTS {
            let proof = accumulator.prove(object);
            assert_eq!(
                verify_proof(&expected, &proof).unwrap(),
                ProofStatus::Member
            );
        }
        observed += 1;
    });
    assert_eq!(observed, 720);
}

#[test]
fn nonmembership_proofs_cover_queries_on_both_sides_of_the_tree() {
    let accumulator = populated(&OBJECTS);
    let root = accumulator.root();
    for missing in [
        "",
        "batch:aardvark",
        "batch:missing",
        "batch:zulu",
        "event:alpha",
    ] {
        let proof = accumulator.prove(missing);
        assert!(matches!(proof.terminal, ProofTerminal::Leaf { .. }));
        assert_eq!(verify_proof(&root, &proof).unwrap(), ProofStatus::NonMember);
    }
}

#[test]
fn proof_tampering_is_rejected() {
    let accumulator = populated(&OBJECTS);
    let root = accumulator.root();
    let proof = accumulator.prove("batch:alpha");
    assert!(!proof.steps.is_empty());

    let mut digest_tamper = proof.clone();
    let mut bytes = *digest_tamper.steps[0].sibling.digest.as_bytes();
    bytes[0] ^= 0x80;
    digest_tamper.steps[0].sibling.digest = Digest::from_bytes(bytes);
    assert!(verify_proof(&root, &digest_tamper).is_err());

    let mut count_tamper = proof.clone();
    count_tamper.steps[0].sibling.count += 1;
    assert!(verify_proof(&root, &count_tamper).is_err());

    let mut range_tamper = proof.clone();
    let mut bytes = *range_tamper.steps[0].sibling.min_key.as_bytes();
    bytes[31] ^= 0x01;
    range_tamper.steps[0].sibling.min_key = Digest::from_bytes(bytes);
    assert!(verify_proof(&root, &range_tamper).is_err());

    let mut branch_tamper = proof.clone();
    branch_tamper.steps[0].branch_bit ^= 0x01;
    assert!(verify_proof(&root, &branch_tamper).is_err());

    let mut direction_tamper = proof.clone();
    direction_tamper.steps[0].direction = match direction_tamper.steps[0].direction {
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
    };
    assert!(verify_proof(&root, &direction_tamper).is_err());

    let mut terminal_tamper = proof.clone();
    terminal_tamper.terminal = ProofTerminal::Leaf {
        object_id: "batch:forged".into(),
    };
    assert!(verify_proof(&root, &terminal_tamper).is_err());

    let mut query_tamper = proof;
    query_tamper.object_id = "batch:forged-query".into();
    assert!(verify_proof(&root, &query_tamper).is_err());
}

#[test]
fn wrong_root_count_and_context_are_rejected() {
    let accumulator = populated(&OBJECTS);
    let root = accumulator.root();
    let proof = accumulator.prove("batch:alpha");

    let mut wrong_count = root;
    wrong_count.count += 1;
    assert_eq!(
        verify_proof(&wrong_count, &proof),
        Err(AccumulatorError::RootMismatch)
    );

    let other = AccumulatorContext::new("scout-ledger", "enterprise:other", "batch").unwrap();
    let mut wrong_context = proof;
    wrong_context.context = other;
    assert!(verify_proof(&root, &wrong_context).is_err());
}

#[test]
fn contexts_separate_identical_object_sets() {
    let batch = populated(&OBJECTS);
    let mut event = Accumulator::new(
        AccumulatorContext::new("scout-ledger", "enterprise:acme", "event").unwrap(),
    );
    for object in OBJECTS {
        event.insert(object).unwrap();
    }
    assert_ne!(batch.root(), event.root());
    assert_ne!(
        batch.context().object_key("shared:id"),
        event.context().object_key("shared:id")
    );
}

#[test]
fn roots_and_keys_have_stable_golden_vectors() {
    let empty = Accumulator::new(context());
    let populated = populated(&OBJECTS[..4]);

    assert_eq!(
        empty.root().digest.to_hex(),
        "564c274e4b2a7af859b4d770b6dd61eb997b40f8535f809aa97d255c05472a37"
    );
    assert_eq!(
        context().object_key("batch:alpha").to_hex(),
        "d27c6f8290e426c5bf4166ec32fe646c2bddac409318da206079175ea8c90cc5"
    );
    assert_eq!(
        populated.root().digest.to_hex(),
        "938f561cf54dce3a78f27c642a9f5a8ec45f24bbd810bb0337242791625a4486"
    );
}

#[test]
fn persistent_mutations_touch_only_the_search_path_and_match_in_memory_roots() {
    let context = context();
    let mut memory = Accumulator::new(context.clone());
    let mut head = AccumulatorHead::empty(&context);
    let mut nodes = BTreeMap::<Digest, StoredNode>::new();
    for object in OBJECTS {
        memory.insert(object).unwrap();
        let mutation = plan_insert(&context, head, object, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .unwrap();
        assert_eq!(mutation.previous, head);
        assert_eq!(mutation.outcome, InsertOutcome::Inserted);
        assert!(mutation.nodes.len() <= 258);
        for node in mutation.nodes {
            let digest = node.digest(&context).unwrap();
            if let Some(existing) = nodes.insert(digest, node.clone()) {
                assert_eq!(existing, node);
            }
        }
        for digest in mutation.obsolete_nodes {
            assert!(nodes.remove(&digest).is_some());
        }
        head = mutation.next;
        assert_eq!(head.root, memory.root());
        assert_eq!(nodes.len(), head.root.count.saturating_mul(2) as usize - 1);
    }

    for object in OBJECTS {
        let proof = prove_persistent(&context, head, object, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .unwrap();
        assert_eq!(
            verify_proof(&head.root, &proof).unwrap(),
            ProofStatus::Member
        );
    }
    let missing = prove_persistent(&context, head, "batch:missing", |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .unwrap();
    assert_eq!(
        verify_proof(&head.root, &missing).unwrap(),
        ProofStatus::NonMember
    );
    let duplicate = plan_insert(&context, head, OBJECTS[0], |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .unwrap();
    assert_eq!(duplicate.outcome, InsertOutcome::AlreadyPresent);
    assert_eq!(duplicate.next, head);
    assert!(duplicate.nodes.is_empty());
    assert!(duplicate.obsolete_nodes.is_empty());
}

#[test]
fn persistent_load_fails_closed_for_missing_or_tampered_nodes() {
    let context = context();
    let empty = AccumulatorHead::empty(&context);
    let first = plan_insert(&context, empty, "batch:alpha", |_| Ok(None)).unwrap();
    let head = first.next;
    assert_eq!(
        plan_insert(&context, head, "batch:bravo", |_| Ok(None)).unwrap_err(),
        AccumulatorError::MissingNode
    );

    let mut forged = first.nodes[0].clone();
    let StoredNode::Leaf { object_id, .. } = &mut forged else {
        panic!("first persistent node must be a leaf")
    };
    *object_id = "batch:forged".into();
    assert!(plan_insert(&context, head, "batch:bravo", |_| Ok(Some(forged.clone()))).is_err());
}

fn visit_permutations(values: &mut [&str], index: usize, visitor: &mut impl FnMut(&[&str])) {
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
