use std::collections::BTreeSet;

use super::*;

const SERVICE_COUNT: usize = 1_200;
const MACHINE_COUNT: usize = 8;

#[test]
fn twelve_hundred_service_fixture_converges_across_eight_machines() {
    let enterprise = enterprise();
    let batches = enterprise_batches(&enterprise);
    let mut forward = EnterpriseGraph::new(enterprise.clone());
    for batch in &batches {
        forward.apply_batch(batch.clone()).unwrap();
    }
    let forward_snapshot = forward.snapshot().unwrap();

    let mut reverse = EnterpriseGraph::new(enterprise);
    for batch in batches.iter().rev() {
        reverse.apply_batch(batch.clone()).unwrap();
    }
    for batch in &batches {
        let report = reverse.apply_batch(batch.clone()).unwrap();
        assert_eq!(report.inserted, 0);
    }
    let reverse_snapshot = reverse.snapshot().unwrap();

    assert_eq!(forward_snapshot.graph_digest, reverse_snapshot.graph_digest);
    assert_eq!(forward_snapshot, reverse_snapshot);
    assert_eq!(forward_snapshot.entities.len(), SERVICE_COUNT * 6 + 3);
    assert_eq!(forward_snapshot.edges.len(), SERVICE_COUNT * 5 + 3);
    assert_eq!(forward_snapshot.simulation_contracts.len(), SERVICE_COUNT);
    assert_eq!(forward_snapshot.coverage.len(), MACHINE_COUNT);
    assert_eq!(forward_snapshot.frontier.len(), MACHINE_COUNT);
    assert_eq!(forward_snapshot.discovery_passes.len(), 2);
    assert!(forward_snapshot.fixed_point);
    assert!(forward_snapshot.conflicts.is_empty());
    assert!(forward_snapshot.completion().complete);

    let changed_id = forward_snapshot
        .entities
        .values()
        .find(|entity| entity.kind == EnterpriseEntityKind::Service)
        .expect("scale fixture service")
        .entity_id
        .clone();
    let added = append_changed_qualification(&mut forward, &changed_id);
    let after_bad = EnterpriseGraph::from_batches(
        forward.enterprise_id().clone(),
        batches.iter().cloned().chain(added.iter().take(1).cloned()),
    )
    .unwrap()
    .snapshot()
    .unwrap();
    assert_eq!(after_bad.entity_history[&changed_id].len(), 1);
    assert!(after_bad.entity_history[&changed_id][0]
        .valid_to_ms
        .is_none());
    let after_first_changed_pass = EnterpriseGraph::from_batches(
        forward.enterprise_id().clone(),
        batches.iter().cloned().chain(added.iter().take(3).cloned()),
    )
    .unwrap()
    .snapshot()
    .unwrap();
    assert_eq!(
        after_first_changed_pass.entity_history[&changed_id].len(),
        1
    );
    assert!(after_first_changed_pass.entity_history[&changed_id][0]
        .valid_to_ms
        .is_none());

    for batch in added.iter().rev() {
        reverse.apply_batch(batch.clone()).unwrap();
    }
    let replay = forward.apply_batch(added.last().unwrap().clone()).unwrap();
    assert_eq!(replay.inserted, 0);
    let qualified = forward.snapshot().unwrap();
    let permuted = reverse.snapshot().unwrap();
    assert_eq!(qualified, permuted);
    let versions = &qualified.entity_history[&changed_id];
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].valid_from_ms, Some(10_000_001));
    assert_eq!(versions[0].valid_to_ms, Some(40_000_001));
    assert_eq!(versions[0].lifecycle, QualifiedLifecycle::Retired);
    assert_eq!(versions[1].valid_from_ms, Some(40_000_001));
    assert_eq!(versions[1].valid_to_ms, None);
    assert_eq!(
        versions[1].classification,
        EnterpriseClassification::Confidential
    );
    assert!(versions[1].labels.contains("qualified-b"));
}

#[test]
fn one_verified_pass_is_not_a_fixed_point() {
    let enterprise = enterprise();
    let batches = enterprise_batches(&enterprise);
    let graph =
        EnterpriseGraph::from_batches(enterprise, batches.into_iter().take(MACHINE_COUNT + 1))
            .unwrap();
    let snapshot = graph.snapshot().unwrap();
    assert_eq!(snapshot.discovery_passes.len(), 1);
    assert!(!snapshot.fixed_point);
    assert!(!snapshot.completion().complete);
}

#[test]
fn newer_topology_activity_stales_the_verified_fixed_point() {
    let enterprise = enterprise();
    let mut graph =
        EnterpriseGraph::from_batches(enterprise.clone(), enterprise_batches(&enterprise)).unwrap();
    assert!(graph.snapshot().unwrap().completion().complete);

    let observation = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "tenant-0", "service:0").unwrap(),
        BTreeSet::from(["unsealed-partial-name".into()]),
        scale_evidence(0, "unsealed-partial-name"),
    )
    .unwrap();
    let entity_id = observation.entity_id.clone();
    let event = EnterpriseEvent::new(
        enterprise.clone(),
        scale_provenance("machine-new", 3, 1),
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap();
    graph
        .apply_batch(EnterpriseBatch::new(enterprise, [event]).unwrap())
        .unwrap();

    let snapshot = graph.snapshot().unwrap();
    assert!(!snapshot.completion().complete);
    assert!(snapshot
        .control_blockers
        .iter()
        .any(|blocker| blocker.contains("newer than the latest verified pass")));
    assert!(!snapshot.entities[&entity_id]
        .labels
        .contains("unsealed-partial-name"));
}

#[test]
fn charter_freshness_expiration_blocks_completion() {
    let enterprise = enterprise();
    let batches = enterprise_batches(&enterprise);
    let graph = EnterpriseGraph::from_batches(enterprise, batches).unwrap();
    let snapshot = graph.snapshot().unwrap();
    let pass = snapshot
        .current_pass_id
        .as_ref()
        .and_then(|pass_id| snapshot.discovery_passes.get(pass_id))
        .unwrap();
    let max_age_ms = snapshot.charter.as_ref().unwrap().max_age_ms;
    assert!(
        !snapshot
            .completion_at(pass.sealed_at_ms + max_age_ms + 1)
            .complete
    );
}

pub(super) fn enterprise_batches(enterprise: &EnterpriseId) -> Vec<EnterpriseBatch> {
    let mut events = vec![Vec::new(); MACHINE_COUNT];
    let mut sequences = [0_u64; MACHINE_COUNT];
    let mut member_entities = vec![BTreeSet::new(); MACHINE_COUNT];
    let mut member_edges = vec![BTreeSet::new(); MACHINE_COUNT];
    let mut critical_runtime_ids = BTreeSet::new();
    let mut first_runtime_id = None;
    for index in 0..SERVICE_COUNT {
        let machine = index % MACHINE_COUNT;
        let machine_id = format!("machine-{machine}");
        let kinds = [
            (EnterpriseEntityKind::Service, "service"),
            (EnterpriseEntityKind::Repository, "repo"),
            (EnterpriseEntityKind::Deployment, "deployment"),
            (EnterpriseEntityKind::Principal, "identity"),
            (EnterpriseEntityKind::Owner, "owner"),
            (EnterpriseEntityKind::Monitor, "monitor"),
        ];
        let mut ids = Vec::new();
        for (kind, prefix) in kinds {
            sequences[machine] += 1;
            let mut observation = GraphEntityObservation::new(
                enterprise,
                kind,
                AuthorityRef::new(
                    if machine.is_multiple_of(2) {
                        "aws"
                    } else {
                        "gcp"
                    },
                    format!("tenant-{machine}"),
                    format!("{prefix}:{index}"),
                )
                .unwrap(),
                BTreeSet::from(["shared-enterprise-service-name".into()]),
                scale_evidence(index, prefix),
            )
            .unwrap();
            observation.critical = kind == EnterpriseEntityKind::Service;
            member_entities[machine].insert(observation.entity_id.clone());
            if kind == EnterpriseEntityKind::Service {
                critical_runtime_ids.insert(observation.entity_id.clone());
                if index == 0 {
                    first_runtime_id = Some(observation.entity_id.clone());
                }
            }
            ids.push(observation.entity_id.clone());
            events[machine].push(
                EnterpriseEvent::new(
                    enterprise.clone(),
                    scale_provenance(&machine_id, 1, sequences[machine]),
                    EnterpriseFact::EntityObserved(observation),
                )
                .unwrap(),
            );
        }
        let edge_specs = [
            (1, 0, EnterpriseEdgeKind::SourceFor),
            (0, 2, EnterpriseEdgeKind::DeploysTo),
            (0, 3, EnterpriseEdgeKind::AuthenticatesVia),
            (0, 4, EnterpriseEdgeKind::OwnedBy),
            (0, 5, EnterpriseEdgeKind::MonitoredBy),
        ];
        for (from, to, kind) in edge_specs {
            sequences[machine] += 1;
            let edge = GraphEdgeObservation::new(
                enterprise,
                ids[from].clone(),
                ids[to].clone(),
                kind,
                None,
                scale_evidence(index, &format!("{kind:?}")),
            )
            .unwrap();
            member_edges[machine].insert(edge.edge_id.clone());
            events[machine].push(
                EnterpriseEvent::new(
                    enterprise.clone(),
                    scale_provenance(&machine_id, 1, sequences[machine]),
                    EnterpriseFact::EdgeObserved(edge),
                )
                .unwrap(),
            );
        }
        sequences[machine] += 1;
        events[machine].push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance(&machine_id, 1, sequences[machine]),
                EnterpriseFact::SimulationContractObserved(SimulationContractObservation {
                    runtime_id: ids[0].clone(),
                    inputs: true,
                    outputs: true,
                    state_effects: true,
                    timeouts: true,
                    retries: true,
                    idempotency: true,
                    failure_behavior: true,
                    observability: true,
                    recovery: true,
                    evidence_digests: scale_evidence(index, "simulation"),
                }),
            )
            .unwrap(),
        );
    }
    let runtime_id = first_runtime_id.expect("service zero runtime");
    let journey_id = add_critical_journey(
        enterprise,
        &runtime_id,
        &mut events[0],
        &mut sequences[0],
        &mut member_entities[0],
        &mut member_edges[0],
    );
    let coverage_keys = (0..MACHINE_COUNT)
        .map(coverage_key)
        .collect::<BTreeSet<_>>();
    sequences[0] += 1;
    events[0].push(
        EnterpriseEvent::new(
            enterprise.clone(),
            scale_provenance("machine-0", 1, sequences[0]),
            EnterpriseFact::DiscoveryCharterObserved(DiscoveryCharterObservation {
                charter_id: "charter:00000000-0000-4000-8000-000000000001".into(),
                revision: 1,
                max_age_ms: 86_400_000,
                supersedes: None,
                required_coverage: coverage_keys,
                critical_journey_ids: BTreeSet::from([journey_id]),
                critical_runtime_ids,
                evidence_digests: scale_evidence(0, "charter"),
            }),
        )
        .unwrap(),
    );
    add_coverage_pass(
        enterprise,
        1,
        &member_entities,
        &member_edges,
        &mut events,
        &mut sequences,
    );

    let mut batches = events
        .into_iter()
        .map(|events| EnterpriseBatch::new(enterprise.clone(), events).unwrap())
        .collect::<Vec<_>>();
    let mut graph = EnterpriseGraph::from_batches(enterprise.clone(), batches.clone()).unwrap();
    let seal_one = graph
        .draft_discovery_pass_seal(
            "charter:00000000-0000-4000-8000-000000000001",
            "epoch-1",
            1,
            None,
            scale_evidence(0, "pass-1"),
        )
        .unwrap();
    let seal_one_id = seal_one.pass_id.clone();
    let seal_one_batch = EnterpriseBatch::new(
        enterprise.clone(),
        [EnterpriseEvent::new(
            enterprise.clone(),
            scale_provenance("coordinator", 1, 1),
            EnterpriseFact::DiscoveryPassSealed(seal_one),
        )
        .unwrap()],
    )
    .unwrap();
    graph.apply_batch(seal_one_batch.clone()).unwrap();
    batches.push(seal_one_batch);

    let mut epoch_two_events = vec![Vec::new(); MACHINE_COUNT];
    let mut epoch_two_sequences = [0_u64; MACHINE_COUNT];
    add_coverage_pass(
        enterprise,
        2,
        &member_entities,
        &member_edges,
        &mut epoch_two_events,
        &mut epoch_two_sequences,
    );
    for batch in epoch_two_events
        .into_iter()
        .map(|events| EnterpriseBatch::new(enterprise.clone(), events).unwrap())
    {
        graph.apply_batch(batch.clone()).unwrap();
        batches.push(batch);
    }
    let seal_two = graph
        .draft_discovery_pass_seal(
            "charter:00000000-0000-4000-8000-000000000001",
            "epoch-2",
            2,
            Some(seal_one_id),
            scale_evidence(0, "pass-2"),
        )
        .unwrap();
    batches.push(
        EnterpriseBatch::new(
            enterprise.clone(),
            [EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance("coordinator", 2, 1),
                EnterpriseFact::DiscoveryPassSealed(seal_two),
            )
            .unwrap()],
        )
        .unwrap(),
    );
    batches
}

fn add_critical_journey(
    enterprise: &EnterpriseId,
    runtime_id: &EnterpriseEntityId,
    events: &mut Vec<EnterpriseEvent>,
    sequence: &mut u64,
    member_entities: &mut BTreeSet<EnterpriseEntityId>,
    member_edges: &mut BTreeSet<EnterpriseEdgeId>,
) -> EnterpriseEntityId {
    let specs = [
        (EnterpriseEntityKind::Actor, "actor:customer"),
        (EnterpriseEntityKind::Journey, "journey:checkout"),
        (EnterpriseEntityKind::Database, "database:orders"),
    ];
    let mut ids = Vec::new();
    for (kind, native_id) in specs {
        *sequence += 1;
        let mut observation = GraphEntityObservation::new(
            enterprise,
            kind,
            AuthorityRef::new("catalog", "business:acme", native_id).unwrap(),
            BTreeSet::from([native_id.into()]),
            scale_evidence(0, native_id),
        )
        .unwrap();
        observation.critical = kind == EnterpriseEntityKind::Journey;
        member_entities.insert(observation.entity_id.clone());
        ids.push(observation.entity_id.clone());
        events.push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance("machine-0", 1, *sequence),
                EnterpriseFact::EntityObserved(observation),
            )
            .unwrap(),
        );
    }
    for (from, to, kind) in [
        (
            ids[0].clone(),
            ids[1].clone(),
            EnterpriseEdgeKind::EntersThrough,
        ),
        (
            ids[1].clone(),
            runtime_id.clone(),
            EnterpriseEdgeKind::Implements,
        ),
        (
            runtime_id.clone(),
            ids[2].clone(),
            EnterpriseEdgeKind::Writes,
        ),
    ] {
        *sequence += 1;
        let edge = GraphEdgeObservation::new(
            enterprise,
            from,
            to,
            kind,
            None,
            scale_evidence(0, &format!("journey:{kind:?}")),
        )
        .unwrap();
        member_edges.insert(edge.edge_id.clone());
        events.push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance("machine-0", 1, *sequence),
                EnterpriseFact::EdgeObserved(edge),
            )
            .unwrap(),
        );
    }
    ids[1].clone()
}

fn add_coverage_pass(
    enterprise: &EnterpriseId,
    epoch: u64,
    member_entities: &[BTreeSet<EnterpriseEntityId>],
    member_edges: &[BTreeSet<EnterpriseEdgeId>],
    events: &mut [Vec<EnterpriseEvent>],
    sequences: &mut [u64; MACHINE_COUNT],
) {
    for machine in 0..MACHINE_COUNT {
        let machine_id = format!("machine-{machine}");
        let key = coverage_key(machine);
        sequences[machine] += 1;
        let mut coverage = CoverageObservation::new(
            enterprise,
            key.clone(),
            CoverageStatus::Supported,
            None,
            member_entities[machine].len() as u64,
            scale_evidence(machine, &format!("coverage-{epoch}")),
        )
        .unwrap();
        coverage.enumerated_edge_count = member_edges[machine].len() as u64;
        events[machine].push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance(&machine_id, epoch, sequences[machine]),
                EnterpriseFact::CoverageObserved(coverage),
            )
            .unwrap(),
        );
        sequences[machine] += 1;
        let mut frontier = FrontierObservation::new(
            enterprise,
            FrontierKey::new(key, None).unwrap(),
            FrontierState::Terminal {
                status: CoverageStatus::Supported,
                reason: "authoritative enumerator reached its final cursor".into(),
            },
            scale_evidence(machine, &format!("frontier-{epoch}")),
        )
        .unwrap();
        frontier.discovered_entity_ids = member_entities[machine].clone();
        frontier.discovered_edge_ids = member_edges[machine].clone();
        events[machine].push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance(&machine_id, epoch, sequences[machine]),
                EnterpriseFact::FrontierObserved(frontier),
            )
            .unwrap(),
        );
    }
}

fn append_changed_qualification(
    graph: &mut EnterpriseGraph,
    changed_id: &EnterpriseEntityId,
) -> Vec<EnterpriseBatch> {
    let enterprise = graph.enterprise_id().clone();
    let before = graph.snapshot().unwrap();
    let prior = &before.entities[changed_id];
    let mut changed = GraphEntityObservation::new(
        &enterprise,
        prior.kind,
        prior.authority.clone(),
        BTreeSet::from(["qualified-b".into()]),
        scale_evidence(0, "qualified-b"),
    )
    .unwrap();
    changed.critical = prior.critical;
    changed.environments = prior.environments.clone();
    changed.classification = EnterpriseClassification::Confidential;
    let bad = EnterpriseBatch::new(
        enterprise.clone(),
        [EnterpriseEvent::new(
            enterprise.clone(),
            scale_provenance("machine-new", 3, 1),
            EnterpriseFact::EntityObserved(changed),
        )
        .unwrap()],
    )
    .unwrap();
    graph.apply_batch(bad.clone()).unwrap();
    let mut batches = vec![bad];

    let pass_a = graph.snapshot().unwrap().current_pass_id.unwrap();
    for (epoch, previous) in [(4, None), (5, Some(()))] {
        let coverage_batch = coverage_batch_from_current(graph, epoch);
        graph.apply_batch(coverage_batch.clone()).unwrap();
        batches.push(coverage_batch);
        let previous_pass_id = if previous.is_some() {
            Some(
                graph
                    .snapshot()
                    .unwrap()
                    .discovery_passes
                    .values()
                    .filter(|pass| pass.discovery_epoch_sequence == 4)
                    .map(|pass| pass.pass_id.clone())
                    .next()
                    .expect("first changed pass"),
            )
        } else {
            Some(pass_a.clone())
        };
        let seal = graph
            .draft_discovery_pass_seal(
                "charter:00000000-0000-4000-8000-000000000001",
                &format!("epoch-{epoch}"),
                epoch,
                previous_pass_id,
                scale_evidence(0, &format!("changed-pass-{epoch}")),
            )
            .unwrap();
        let seal_batch = EnterpriseBatch::new(
            enterprise.clone(),
            [EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance("coordinator", epoch, 1),
                EnterpriseFact::DiscoveryPassSealed(seal),
            )
            .unwrap()],
        )
        .unwrap();
        graph.apply_batch(seal_batch.clone()).unwrap();
        batches.push(seal_batch);
    }
    batches
}

fn coverage_batch_from_current(graph: &EnterpriseGraph, epoch: u64) -> EnterpriseBatch {
    let enterprise = graph.enterprise_id().clone();
    let snapshot = graph.snapshot().unwrap();
    let mut events = Vec::new();
    for (machine, frontier) in snapshot.frontier.values().enumerate() {
        let machine_id = format!("machine-{machine}");
        let key = frontier.key.coverage.clone();
        let mut coverage = CoverageObservation::new(
            &enterprise,
            key.clone(),
            CoverageStatus::Supported,
            None,
            frontier.discovered_entity_ids.len() as u64,
            scale_evidence(machine, &format!("changed-coverage-{epoch}")),
        )
        .unwrap();
        coverage.enumerated_edge_count = frontier.discovered_edge_ids.len() as u64;
        events.push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance(&machine_id, epoch, 1),
                EnterpriseFact::CoverageObserved(coverage),
            )
            .unwrap(),
        );
        let mut next = FrontierObservation::new(
            &enterprise,
            FrontierKey::new(key, None).unwrap(),
            FrontierState::Terminal {
                status: CoverageStatus::Supported,
                reason: "authoritative enumerator reached its final cursor".into(),
            },
            scale_evidence(machine, &format!("changed-frontier-{epoch}")),
        )
        .unwrap();
        next.discovered_entity_ids = frontier.discovered_entity_ids.clone();
        next.discovered_edge_ids = frontier.discovered_edge_ids.clone();
        events.push(
            EnterpriseEvent::new(
                enterprise.clone(),
                scale_provenance(&machine_id, epoch, 2),
                EnterpriseFact::FrontierObserved(next),
            )
            .unwrap(),
        );
    }
    EnterpriseBatch::new(enterprise, events).unwrap()
}

fn coverage_key(machine: usize) -> CoverageKey {
    CoverageKey::new(
        if machine.is_multiple_of(2) {
            "aws"
        } else {
            "gcp"
        },
        "auth-read-only",
        format!("tenant-{machine}"),
        "all-regions",
        "runtime",
    )
    .unwrap()
}

fn scale_provenance(machine: &str, epoch: u64, sequence: u64) -> EnterpriseProvenance {
    EnterpriseProvenance {
        machine_id: machine.into(),
        run_id: format!("enterprise-scale-{machine}-{epoch}"),
        adapter_instance_id: format!("adapter-{machine}"),
        auth_context_id: "auth-read-only".into(),
        discovery_epoch: format!("epoch-{epoch}"),
        discovery_epoch_sequence: epoch,
        source_sequence: sequence,
        observed_at_ms: epoch * 10_000_000 + sequence,
        source_fingerprint: "f".repeat(64),
    }
}

fn scale_evidence(index: usize, salt: &str) -> BTreeSet<String> {
    use sha2::{Digest, Sha256};
    BTreeSet::from([format!(
        "{:x}",
        Sha256::digest(format!("{index}:{salt}").as_bytes())
    )])
}
