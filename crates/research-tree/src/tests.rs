use super::*;

fn tree(nodes: usize, steps: usize) -> ResearchTree {
    ResearchTree::new(
        Identity {
            scope_id: "design-security-tree".into(),
            source_id: "local-implementation/library".into(),
        },
        Limits {
            max_nodes: nodes,
            max_steps: steps,
        },
    )
    .unwrap()
}
fn proposed(key: &str, parent: Option<&str>, dependencies: &[&str]) -> Event {
    Event::Proposed {
        key: key.into(),
        parent: parent.map(str::to_owned),
        links: vec![],
        dependencies: dependencies.iter().map(|s| (*s).into()).collect(),
        evidence_refs: vec!["src/auth.rs:12".into()],
        rationale: "Inspect the authorization boundary".into(),
        assessment: Assessment {
            surface: SurfaceKind::Hypothesis,
            in_scope: true,
            requires_validation: true,
            priority: 0.5,
            novelty: 0.5,
            confidence: 0.5,
            impact: 0.5,
        },
    }
}
fn record(key: &str, outcome: Outcome) -> Event {
    Event::Recorded {
        key: key.into(),
        outcome,
        evidence_refs: vec!["receipt:negative-control".into()],
        rationale: "Caller observed result".into(),
    }
}

#[test]
fn journal_replay_reconstructs_identical_work_and_rejects_unknown_fields() {
    let mut t = tree(8, 8);
    t.apply(proposed("root", None, &[])).unwrap();
    t.apply(proposed("child", Some("root"), &["root"])).unwrap();
    assert_eq!(t.select().unwrap().unwrap().key, "root");
    t.apply(record("root", Outcome::Supports)).unwrap();
    t.select().unwrap();
    t.apply(record("child", Outcome::Blocked)).unwrap();
    let wire = serde_json::to_string(&t.journal()).unwrap();
    let journal: Journal = serde_json::from_str(&wire).unwrap();
    assert_eq!(
        ResearchTree::replay(journal.identity, journal.limits, journal.events).unwrap(),
        t
    );
    assert!(
        serde_json::from_str::<Event>(r#"{"type":"started","key":"root","verified":true}"#)
            .is_err()
    );
}

#[test]
fn shared_semantic_key_keeps_one_node_and_crosslinks_without_repeating_terminal_work() {
    let mut t = tree(3, 8);
    for key in ["a", "b"] {
        t.apply(proposed(key, None, &[])).unwrap();
    }
    t.apply(proposed("shared", Some("a"), &[])).unwrap();
    t.apply(proposed("shared", Some("b"), &[])).unwrap();
    assert_eq!(t.nodes().len(), 3);
    assert_eq!(t.nodes()["shared"].parent.as_deref(), Some("a"));
    assert_eq!(t.nodes()["shared"].links, ["b"]);
    for _ in 0..3 {
        let selected = t.select().unwrap().unwrap();
        t.apply(record(&selected.key, Outcome::Refutes)).unwrap();
    }
    t.apply(proposed("shared", Some("b"), &[])).unwrap();
    assert!(t.select().unwrap().is_none());
    assert_eq!(t.steps_used(), 3);
    assert_eq!(t.nodes()["shared"].status, Status::Refuted);
}

#[test]
fn budgets_scope_dependencies_and_ties_are_deterministic() {
    let mut t = tree(4, 1);
    t.apply(proposed("z", None, &[])).unwrap();
    t.apply(proposed("a", None, &[])).unwrap();
    let mut out = proposed("outside", None, &[]);
    if let Event::Proposed { assessment, .. } = &mut out {
        assessment.in_scope = false;
        assessment.priority = 1.0;
    }
    t.apply(out).unwrap();
    t.apply(proposed("child", None, &["a"])).unwrap();
    assert_eq!(
        t.ranked()
            .iter()
            .map(|h| h.node_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(t.select().unwrap().unwrap().key, "a");
    t.apply(record("a", Outcome::Inconclusive)).unwrap();
    assert!(matches!(t.select(), Err(TreeError::Budget(_))));
    assert!(t.ranked().is_empty());
    assert!(matches!(
        t.apply(proposed("fifth", None, &[])),
        Err(TreeError::Budget(_))
    ));
}

#[test]
fn failed_transitions_are_atomic_including_partially_merged_reference_overflow() {
    let mut t = tree(3, 4);
    t.apply(proposed("a", None, &[])).unwrap();
    t.apply(proposed("b", None, &[])).unwrap();
    let baseline = t.clone();
    for event in [
        Event::Started { key: "b".into() },
        record("a", Outcome::Supports),
        proposed("bad", Some("missing"), &[]),
    ] {
        assert!(t.apply(event).is_err());
        assert_eq!(t, baseline);
    }
    let mut duplicate = proposed("a", Some("b"), &[]);
    if let Event::Proposed { evidence_refs, .. } = &mut duplicate {
        *evidence_refs = (0..MAX_REFERENCES).map(|i| format!("new:{i}")).collect();
    }
    assert!(t.apply(duplicate).is_err());
    assert_eq!(t, baseline);
}

#[test]
fn terminal_work_reopens_only_with_new_evidence_and_charges_another_attempt() {
    let mut t = tree(1, 3);
    t.apply(proposed("a", None, &[])).unwrap();
    t.select().unwrap();
    let mut supports = record("a", Outcome::Supports);
    if let Event::Recorded { evidence_refs, .. } = &mut supports {
        evidence_refs.clear();
    }
    assert!(t.apply(supports).is_err());
    t.apply(record("a", Outcome::Blocked)).unwrap();
    let baseline = t.clone();
    assert!(t
        .apply(Event::Reopened {
            key: "a".into(),
            evidence_refs: vec!["src/auth.rs:12".into()],
            rationale: "try again".into()
        })
        .is_err());
    assert_eq!(t, baseline);
    t.apply(Event::Reopened {
        key: "a".into(),
        evidence_refs: vec!["receipt:fixed-environment".into()],
        rationale: "Environment repaired".into(),
    })
    .unwrap();
    assert_eq!(t.select().unwrap().unwrap().status, Status::Active);
    assert_eq!(t.steps_used(), 2);
}

#[test]
fn hard_limits_reject_invalid_settings_and_long_fields() {
    assert!(ResearchTree::new(
        Identity {
            scope_id: "scope".into(),
            source_id: "source".into()
        },
        Limits {
            max_nodes: 257,
            max_steps: 1
        }
    )
    .is_err());
    let mut t = tree(1, 1);
    assert!(t.apply(proposed(&"a".repeat(161), None, &[])).is_err());
    t.apply(proposed("a", None, &[])).unwrap();
    for _ in 1..MAX_EVENTS {
        t.apply(proposed("a", None, &[])).unwrap();
    }
    let before = t.clone();
    assert!(matches!(
        t.apply(proposed("a", None, &[])),
        Err(TreeError::Budget(_))
    ));
    assert_eq!(t, before);
}

#[test]
fn dependency_sets_deduplicate_and_blocked_or_pruned_work_does_not_unlock_children() {
    let mut t = tree(4, 8);
    t.apply(proposed("parent", None, &[])).unwrap();
    t.apply(proposed("child", None, &["parent", "parent"]))
        .unwrap();
    t.apply(proposed("child", None, &["parent"])).unwrap();
    assert_eq!(t.nodes()["child"].dependencies, ["parent"]);
    t.select().unwrap();
    t.apply(record("parent", Outcome::Blocked)).unwrap();
    assert!(t.select().unwrap().is_none());
    t.apply(Event::Pruned {
        key: "parent".into(),
        rationale: "No longer actionable".into(),
    })
    .unwrap();
    assert!(t.select().unwrap().is_none());
    t.apply(Event::Reopened {
        key: "parent".into(),
        evidence_refs: vec!["new-source".into()],
        rationale: "New source permits investigation".into(),
    })
    .unwrap();
    t.select().unwrap();
    t.apply(record("parent", Outcome::Expands)).unwrap();
    assert_eq!(t.select().unwrap().unwrap().key, "child");
}

#[test]
fn new_evidence_can_rerank_pending_work_without_changing_scope() {
    let mut t = tree(3, 3);
    t.apply(proposed("a", None, &[])).unwrap();
    t.apply(proposed("b", None, &[])).unwrap();
    let mut outside = proposed("outside", None, &[]);
    if let Event::Proposed { assessment, .. } = &mut outside {
        assessment.in_scope = false;
    }
    t.apply(outside).unwrap();
    assert_eq!(t.ranked()[0].node_id, "a");
    let mut stronger = t.nodes()["b"].assessment.clone();
    stronger.priority = 1.0;
    let baseline = t.clone();
    for refs in [vec![], vec!["src/auth.rs:12".into()]] {
        assert!(t
            .apply(Event::Reassessed {
                key: "b".into(),
                evidence_refs: refs,
                rationale: "stronger estimate".into(),
                assessment: stronger.clone()
            })
            .is_err());
        assert_eq!(t, baseline);
    }
    assert!(t
        .apply(Event::Reassessed {
            key: "outside".into(),
            evidence_refs: vec!["new-ref".into()],
            rationale: "widen scope".into(),
            assessment: stronger.clone()
        })
        .is_err());
    assert_eq!(t, baseline);
    t.apply(Event::Reassessed {
        key: "b".into(),
        evidence_refs: vec!["new-ref".into()],
        rationale: "new observed path increases priority".into(),
        assessment: stronger.clone(),
    })
    .unwrap();
    assert_eq!(t.ranked()[0].node_id, "b");
    t.select().unwrap();
    let active = t.clone();
    assert!(t
        .apply(Event::Reassessed {
            key: "b".into(),
            evidence_refs: vec!["another-ref".into()],
            rationale: "active".into(),
            assessment: stronger
        })
        .is_err());
    assert_eq!(t, active);
    let journal = t.journal();
    assert_eq!(
        ResearchTree::replay(journal.identity, journal.limits, journal.events).unwrap(),
        t
    );
}

#[test]
fn context_keeps_requested_branch_with_shared_links_outside_first_graph_page() {
    let mut t = tree(64, 64);
    for i in 0..40 {
        t.apply(proposed(&format!("unrelated-{i:02}"), None, &[]))
            .unwrap();
    }
    t.apply(proposed("root", None, &[])).unwrap();
    t.apply(proposed("parent", Some("root"), &[])).unwrap();
    t.apply(proposed("dependency", None, &[])).unwrap();
    t.apply(proposed("shared", None, &[])).unwrap();
    let mut child = proposed("selected", Some("parent"), &["dependency"]);
    if let Event::Proposed { links, .. } = &mut child {
        *links = vec!["shared".into(), "root".into(), "dependency".into()];
    }
    t.apply(child).unwrap();
    let keys = |limit| {
        t.context("selected", limit)
            .unwrap()
            .into_iter()
            .map(|node| node.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        keys(16),
        ["selected", "parent", "root", "dependency", "shared"]
    );
    assert_eq!(keys(2), ["selected", "parent"]);
    assert_eq!(keys(1), ["selected"]);
    assert!(matches!(
        t.context("missing", 16),
        Err(TreeError::UnknownNode(_))
    ));
    for limit in [0, 33, usize::MAX] {
        assert!(t.context("selected", limit).is_err());
    }
}

#[test]
fn finite_journal_reserves_terminal_outcomes_and_replay_rejects_stranded_start() {
    let mut t = tree(1, 8);
    for _ in 0..MAX_EVENTS - 2 {
        t.apply(proposed("a", None, &[])).unwrap();
    }
    t.select().unwrap().unwrap();
    let active = t.clone();
    assert!(t.apply(proposed("a", None, &[])).is_err());
    assert_eq!(t, active);
    t.apply(record("a", Outcome::Blocked)).unwrap();
    assert_eq!(t.events().len(), MAX_EVENTS);
    let mut journal = t.journal();
    journal.events.pop();
    journal.events.pop();
    journal.events.push(proposed("a", None, &[]));
    journal.events.push(Event::Started { key: "a".into() });
    assert!(ResearchTree::replay(journal.identity, journal.limits, journal.events).is_err());
}
