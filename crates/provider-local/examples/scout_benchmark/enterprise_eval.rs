use std::collections::BTreeSet;
use std::time::Instant;

use agent_orchestration::{
    AuthorityRef, CoverageKey, CoverageObservation, CoverageStatus, DiscoveryCharterObservation,
    EnterpriseBatch, EnterpriseEdgeId, EnterpriseEdgeKind, EnterpriseEntityId,
    EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact, EnterpriseGrantScope, EnterpriseGraph,
    EnterpriseId, EnterpriseProvenance, EnterpriseSignedBatch, EnterpriseSignerGrant,
    EnterpriseSignerRole, EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest,
    FrontierKey, FrontierObservation, FrontierState, GraphEdgeObservation, GraphEntityObservation,
    SimulationContractObservation, MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "enterprise_eval/index_fixture.rs"]
mod index_fixture;

struct FixtureState {
    events: Vec<Vec<EnterpriseEvent>>,
    sequences: Vec<u64>,
    member_entities: Vec<BTreeSet<EnterpriseEntityId>>,
    member_edges: Vec<BTreeSet<EnterpriseEdgeId>>,
    critical_runtime_ids: BTreeSet<EnterpriseEntityId>,
    first_runtime_id: Option<EnterpriseEntityId>,
}

pub fn multi_machine_scale(
    service_count: usize,
    machine_count: usize,
) -> Result<(String, Value), String> {
    let enterprise = EnterpriseId::new("benchmark-enterprise")?;
    let started = Instant::now();
    let batches = batches(&enterprise, service_count, machine_count)?;
    let batch_count = batches.len();
    let (trust_chain, signed_batches) = sign_batches(&enterprise, &batches)?;
    drop(batches);
    let build_and_sign_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let mut forward = EnterpriseGraph::new(enterprise.clone());
    for envelope in &signed_batches {
        let verified = trust_chain.verify_signed_batch(envelope.clone())?;
        forward.apply_batch(verified.batch().clone())?;
    }
    let forward_snapshot = forward.snapshot()?;
    drop(forward);
    let forward_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let mut reverse = EnterpriseGraph::new(enterprise);
    for envelope in signed_batches.iter().rev() {
        let verified = trust_chain.verify_signed_batch(envelope.clone())?;
        reverse.apply_batch(verified.batch().clone())?;
    }
    let duplicate = reverse.apply_batch(
        trust_chain
            .verify_signed_batch(signed_batches[0].clone())?
            .batch()
            .clone(),
    )?;
    let reverse_snapshot = reverse.snapshot()?;
    drop(reverse);
    let reverse_ms = started.elapsed().as_millis();

    if forward_snapshot != reverse_snapshot {
        return Err("enterprise graph changed with batch delivery order".into());
    }
    if duplicate.inserted != 0 {
        return Err("duplicate enterprise batch was not idempotent".into());
    }
    let completion = forward_snapshot.completion();
    if !completion.complete {
        return Err(format!(
            "enterprise scale fixture is incomplete: {:?}",
            completion.blockers
        ));
    }
    let expected_entities = service_count * 6 + 3;
    let expected_edges = service_count * 5 + 3;
    if forward_snapshot.entities.len() != expected_entities
        || forward_snapshot.edges.len() != expected_edges
        || forward_snapshot.simulation_contracts.len() != service_count
        || forward_snapshot.coverage.len() != machine_count
        || forward_snapshot.frontier.len() != machine_count
        || forward_snapshot.discovery_passes.len() != 2
        || !forward_snapshot.fixed_point
    {
        return Err("enterprise scale fixture materialized unexpected counts".into());
    }
    let index = index_fixture::index_fixture(&trust_chain, &signed_batches)?;
    let authentication_root = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(
                &signed_batches
                    .iter()
                    .map(|envelope| {
                        (
                            &envelope.batch.batch_id,
                            &envelope.manifest_id,
                            &envelope.grant.grant_id,
                            &envelope.signer_id,
                            &envelope.signature,
                        )
                    })
                    .collect::<Vec<_>>()
            )
            .map_err(|error| error.to_string())?
        )
    );
    let semantic_payload = json!({
        "services": service_count,
        "machines": machine_count,
        "batches": batch_count,
        "authenticated_batches": signed_batches.len(),
        "trust_anchor": trust_chain.anchor_manifest_id,
        "authentication_root": authentication_root,
        "indexed_event_root": index.event_root,
        "indexed_graph_digest": index.graph_digest,
        "indexed_event_set_root_v1": index.event_set_root_v1,
        "indexed_projection_map_root_v2": index.projection_map_root_v2,
        "indexed_enterprise_snapshot_root_v2": index.enterprise_snapshot_root_v2,
        "warm_envelope_rows_read": index.warm_envelope_rows_read,
        "indexed_page_size": index.page_size,
        "checkpoint_id": index.checkpoint_id,
        "checkpoint_sequence": index.checkpoint_sequence,
        "checkpoint_covers_current_ledger": index.checkpoint_covers_current_ledger,
        "checkpoint_delta_batch_count": index.checkpoint_delta_batch_count,
        "checkpoint_chain_membership_entries": index.checkpoint_chain_membership_entries,
        "checkpoint_chain_bytes": index.checkpoint_chain_bytes,
        "projection_rows_written": index.projection_rows_written,
        "projection_rows_deleted": index.projection_rows_deleted,
        "supplemental_rows_written": index.supplemental_rows_written,
        "supplemental_rows_deleted": index.supplemental_rows_deleted,
        "projection_total_rows": index.projection_total_rows,
        "index_bytes": index.index_bytes,
        "index_page_count": index.index_page_count,
        "index_freelist_pages": index.index_freelist_pages,
        "index_table_bytes": index.index_table_bytes,
        "events": forward_snapshot.event_count,
        "entities": forward_snapshot.entities.len(),
        "edges": forward_snapshot.edges.len(),
        "simulation_contracts": forward_snapshot.simulation_contracts.len(),
        "coverage_cells": forward_snapshot.coverage.len(),
        "frontier_tasks": forward_snapshot.frontier.len(),
        "discovery_passes": forward_snapshot.discovery_passes.len(),
        "fixed_point": forward_snapshot.fixed_point,
        "conflicts": forward_snapshot.conflicts.len(),
        "event_root": forward_snapshot.event_root,
        "graph_digest": forward_snapshot.graph_digest,
        "duplicate_inserted": duplicate.inserted,
        "complete": completion.complete
    });
    let semantic_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&semantic_payload).map_err(|error| error.to_string())?)
    );

    let mut details = json!({
            "services": service_count,
            "machines": machine_count,
            "batches": batch_count,
            "authenticated_batches": signed_batches.len(),
            "trust_anchor": trust_chain.anchor_manifest_id,
            "authentication_root": authentication_root,
            "indexed_event_root": index.event_root,
            "indexed_graph_digest": index.graph_digest,
            "warm_envelope_rows_read": index.warm_envelope_rows_read,
            "indexed_page_size": index.page_size,
            "index_rebuild_ms": index.rebuild_ms,
            "warm_status_ms": index.warm_status_ms,
            "index_bytes": index.index_bytes,
            "projection_rows_written": index.projection_rows_written,
            "projection_rows_deleted": index.projection_rows_deleted,
            "projection_total_rows": index.projection_total_rows,
            "checkpoint_id": index.checkpoint_id,
            "checkpoint_sequence": index.checkpoint_sequence,
            "checkpoint_covers_current_ledger": index.checkpoint_covers_current_ledger,
            "checkpoint_delta_batch_count": index.checkpoint_delta_batch_count,
            "checkpoint_chain_membership_entries": index.checkpoint_chain_membership_entries,
            "checkpoint_chain_bytes": index.checkpoint_chain_bytes,
            "events": forward_snapshot.event_count,
            "entities": forward_snapshot.entities.len(),
            "edges": forward_snapshot.edges.len(),
            "simulation_contracts": forward_snapshot.simulation_contracts.len(),
            "coverage_cells": forward_snapshot.coverage.len(),
            "frontier_tasks": forward_snapshot.frontier.len(),
            "discovery_passes": forward_snapshot.discovery_passes.len(),
            "fixed_point": forward_snapshot.fixed_point,
            "conflicts": forward_snapshot.conflicts.len(),
            "event_root": forward_snapshot.event_root,
            "graph_digest": forward_snapshot.graph_digest,
            "duplicate_inserted": duplicate.inserted,
            "build_and_sign_ms": build_and_sign_ms,
            "forward_materialize_ms": forward_ms,
            "reverse_materialize_ms": reverse_ms,
            "complete": completion.complete,
            "semantic_sha256": semantic_sha256
    });
    let Value::Object(detail_fields) = &mut details else {
        return Err("enterprise benchmark details are not an object".into());
    };
    detail_fields.insert(
        "indexed_event_set_root_v1".into(),
        json!(index.event_set_root_v1),
    );
    detail_fields.insert(
        "indexed_projection_map_root_v2".into(),
        json!(index.projection_map_root_v2),
    );
    detail_fields.insert(
        "indexed_enterprise_snapshot_root_v2".into(),
        json!(index.enterprise_snapshot_root_v2),
    );
    detail_fields.insert(
        "supplemental_rows_written".into(),
        json!(index.supplemental_rows_written),
    );
    detail_fields.insert(
        "supplemental_rows_deleted".into(),
        json!(index.supplemental_rows_deleted),
    );
    detail_fields.insert("index_page_count".into(), json!(index.index_page_count));
    detail_fields.insert(
        "index_freelist_pages".into(),
        json!(index.index_freelist_pages),
    );
    detail_fields.insert("index_table_bytes".into(), json!(index.index_table_bytes));

    Ok((
        format!(
            "{service_count} services from {machine_count} machines converged deterministically"
        ),
        details,
    ))
}

pub fn signed_scale_fixture(
    service_count: usize,
    machine_count: usize,
) -> Result<(EnterpriseTrustChain, Vec<EnterpriseSignedBatch>), String> {
    let enterprise = EnterpriseId::new("benchmark-enterprise")?;
    let batches = batches(&enterprise, service_count, machine_count)?;
    sign_batches(&enterprise, &batches)
}

/// Isolates the resident cost of the authoritative event graph and one exact
/// snapshot without involving SQLite or the central-ingest benchmark.
///
/// `prepare_affected_projection` models the old eager-cache resident shape by
/// asking the graph to initialize its affected-row event index through a
/// duplicate, logically idempotent batch. The default replay path leaves that
/// secondary index absent until an incremental projection is actually needed.
#[allow(dead_code)]
pub fn graph_memory_profile(
    service_count: usize,
    machine_count: usize,
    prepare_affected_projection: bool,
) -> Result<Value, String> {
    let enterprise = EnterpriseId::new("benchmark-enterprise")?;
    let batches = batches(&enterprise, service_count, machine_count)?;
    let (trust_chain, signed_batches) = sign_batches(&enterprise, &batches)?;
    drop(batches);

    let mut graph = EnterpriseGraph::new(enterprise);
    for envelope in &signed_batches {
        let verified = trust_chain.verify_signed_batch(envelope.clone())?;
        graph.apply_batch(verified.batch().clone())?;
    }
    if prepare_affected_projection {
        let mut cursor = graph.projection_cursor()?;
        let duplicate = graph.apply_batch_affected(
            &mut cursor,
            trust_chain
                .verify_signed_batch(signed_batches[0].clone())?
                .batch()
                .clone(),
        )?;
        if duplicate.merge.inserted != 0
            || duplicate.work.inserted_events != 0
            || duplicate.global_metadata_changed
        {
            return Err("projection-cache preparation changed the logical graph".into());
        }
    }

    let snapshot = graph.snapshot()?;
    let evidence = json!({
        "services": service_count,
        "machines": machine_count,
        "batches": signed_batches.len(),
        "events": snapshot.event_count,
        "entities": snapshot.entities.len(),
        "edges": snapshot.edges.len(),
        "simulation_contracts": snapshot.simulation_contracts.len(),
        "event_root": snapshot.event_root,
        "graph_digest": snapshot.graph_digest,
        "affected_projection_cache_prepared": prepare_affected_projection,
    });
    std::hint::black_box((&graph, &snapshot, &signed_batches));
    Ok(evidence)
}

fn sign_batches(
    enterprise: &EnterpriseId,
    batches: &[EnterpriseBatch],
) -> Result<(EnterpriseTrustChain, Vec<EnterpriseSignedBatch>), String> {
    let coordinator = EnterpriseSigningKey::from_seed([0x42; 32]);
    let root = EnterpriseTrustManifest::initial(
        enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000042".into(),
        1,
        1_000_000_000,
        &coordinator,
    )?;
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: root.manifest_id.clone(),
        manifests: vec![root.clone()],
    };
    let mut envelopes = Vec::with_capacity(batches.len());
    for (index, batch) in batches.iter().enumerate() {
        let coordinator_only = batch.events.iter().any(|event| {
            matches!(
                event.fact,
                EnterpriseFact::DiscoveryCharterObserved(_)
                    | EnterpriseFact::DiscoveryPassSealed(_)
                    | EnterpriseFact::ObservationRetracted { .. }
            )
        });
        let worker;
        let signer = if coordinator_only {
            &coordinator
        } else {
            let machine = &batch.events[0].provenance.machine_id;
            let seed: [u8; 32] =
                Sha256::digest(format!("scout-benchmark-worker/v1\0{machine}")).into();
            worker = EnterpriseSigningKey::from_seed(seed);
            &worker
        };
        let roles = if coordinator_only {
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ])
        } else {
            BTreeSet::from([EnterpriseSignerRole::Collector])
        };
        let grant = EnterpriseSignerGrant::issue(
            &root,
            signer.signer_id(),
            signer.public_key_hex(),
            roles,
            batch_scope(batch)?,
            1,
            1_000_000_000,
            &[&coordinator],
        )?;
        let envelope = EnterpriseSignedBatch::sign(
            batch.clone(),
            &root,
            grant,
            10_000 + index as u64,
            signer,
        )?;
        chain.verify_signed_batch(envelope.clone())?;
        envelopes.push(envelope);
    }
    Ok((chain, envelopes))
}

fn batch_scope(batch: &EnterpriseBatch) -> Result<EnterpriseGrantScope, String> {
    let first = batch
        .events
        .first()
        .ok_or_else(|| "benchmark batch is empty".to_string())?;
    let mut first_source_sequence = u64::MAX;
    let mut last_source_sequence = 0;
    for event in &batch.events {
        if event.provenance.machine_id != first.provenance.machine_id
            || event.provenance.run_id != first.provenance.run_id
            || event.provenance.adapter_instance_id != first.provenance.adapter_instance_id
            || event.provenance.auth_context_id != first.provenance.auth_context_id
            || event.provenance.discovery_epoch != first.provenance.discovery_epoch
            || event.provenance.discovery_epoch_sequence
                != first.provenance.discovery_epoch_sequence
        {
            return Err("benchmark batch mixes signer scopes".into());
        }
        first_source_sequence = first_source_sequence.min(event.provenance.source_sequence);
        last_source_sequence = last_source_sequence.max(event.provenance.source_sequence);
    }
    Ok(EnterpriseGrantScope {
        machine_id: first.provenance.machine_id.clone(),
        run_id: first.provenance.run_id.clone(),
        adapter_instance_id: first.provenance.adapter_instance_id.clone(),
        auth_context_id: first.provenance.auth_context_id.clone(),
        discovery_epoch: first.provenance.discovery_epoch.clone(),
        discovery_epoch_sequence: first.provenance.discovery_epoch_sequence,
        first_source_sequence,
        last_source_sequence,
    })
}

fn batches(
    enterprise: &EnterpriseId,
    service_count: usize,
    machine_count: usize,
) -> Result<Vec<EnterpriseBatch>, String> {
    let mut fixture = FixtureState {
        events: vec![Vec::new(); machine_count],
        sequences: vec![0_u64; machine_count],
        member_entities: vec![BTreeSet::new(); machine_count],
        member_edges: vec![BTreeSet::new(); machine_count],
        critical_runtime_ids: BTreeSet::new(),
        first_runtime_id: None,
    };
    for index in 0..service_count {
        add_service(enterprise, index, machine_count, &mut fixture)?;
    }
    let FixtureState {
        mut events,
        mut sequences,
        mut member_entities,
        mut member_edges,
        critical_runtime_ids,
        first_runtime_id,
    } = fixture;
    let journey_id = add_critical_journey(
        enterprise,
        &first_runtime_id.ok_or_else(|| "missing first runtime".to_string())?,
        &mut events[0],
        &mut sequences[0],
        &mut member_entities[0],
        &mut member_edges[0],
    )?;
    sequences[0] += 1;
    events[0].push(EnterpriseEvent::new(
        enterprise.clone(),
        provenance("machine-0", 1, sequences[0]),
        EnterpriseFact::DiscoveryCharterObserved(DiscoveryCharterObservation {
            charter_id: "charter:00000000-0000-4000-8000-000000000001".into(),
            revision: 1,
            max_age_ms: 86_400_000,
            supersedes: None,
            required_coverage: (0..machine_count).map(coverage_key).collect(),
            critical_journey_ids: BTreeSet::from([journey_id]),
            critical_runtime_ids,
            evidence_digests: evidence(0, "charter"),
        }),
    )?);
    add_coverage_pass(
        enterprise,
        1,
        &member_entities,
        &member_edges,
        &mut events,
        &mut sequences,
    )?;
    let mut batches = chunk_batches(enterprise, events)?;
    let mut graph = EnterpriseGraph::from_batches(enterprise.clone(), batches.clone())?;
    let seal_one = graph.draft_discovery_pass_seal(
        "charter:00000000-0000-4000-8000-000000000001",
        "epoch-1",
        1,
        None,
        evidence(0, "pass-1"),
    )?;
    let seal_one_id = seal_one.pass_id.clone();
    let seal_one_batch = EnterpriseBatch::new(
        enterprise.clone(),
        [EnterpriseEvent::new(
            enterprise.clone(),
            provenance("coordinator", 1, 1),
            EnterpriseFact::DiscoveryPassSealed(seal_one),
        )?],
    )?;
    graph.apply_batch(seal_one_batch.clone())?;
    batches.push(seal_one_batch);

    let mut epoch_two_events = vec![Vec::new(); machine_count];
    let mut epoch_two_sequences = vec![0_u64; machine_count];
    add_coverage_pass(
        enterprise,
        2,
        &member_entities,
        &member_edges,
        &mut epoch_two_events,
        &mut epoch_two_sequences,
    )?;
    for batch in epoch_two_events
        .into_iter()
        .map(|events| EnterpriseBatch::new(enterprise.clone(), events))
    {
        let batch = batch?;
        graph.apply_batch(batch.clone())?;
        batches.push(batch);
    }
    let seal_two = graph.draft_discovery_pass_seal(
        "charter:00000000-0000-4000-8000-000000000001",
        "epoch-2",
        2,
        Some(seal_one_id),
        evidence(0, "pass-2"),
    )?;
    batches.push(EnterpriseBatch::new(
        enterprise.clone(),
        [EnterpriseEvent::new(
            enterprise.clone(),
            provenance("coordinator", 2, 1),
            EnterpriseFact::DiscoveryPassSealed(seal_two),
        )?],
    )?);
    Ok(batches)
}

fn chunk_batches(
    enterprise: &EnterpriseId,
    machine_events: Vec<Vec<EnterpriseEvent>>,
) -> Result<Vec<EnterpriseBatch>, String> {
    let mut batches = Vec::new();
    for events in machine_events {
        for chunk in events.chunks(MAX_ENTERPRISE_EVENTS_PER_BATCH) {
            batches.push(EnterpriseBatch::new(enterprise.clone(), chunk.to_vec())?);
        }
    }
    Ok(batches)
}

fn add_service(
    enterprise: &EnterpriseId,
    index: usize,
    machine_count: usize,
    fixture: &mut FixtureState,
) -> Result<(), String> {
    let machine = index % machine_count;
    let machine_id = format!("machine-{machine}");
    let adapter = if machine % 2 == 0 { "aws" } else { "gcp" };
    let entity_specs = [
        (EnterpriseEntityKind::Service, "service"),
        (EnterpriseEntityKind::Repository, "repo"),
        (EnterpriseEntityKind::Deployment, "deployment"),
        (EnterpriseEntityKind::Principal, "identity"),
        (EnterpriseEntityKind::Owner, "owner"),
        (EnterpriseEntityKind::Monitor, "monitor"),
    ];
    let mut ids = Vec::new();
    for (kind, prefix) in entity_specs {
        fixture.sequences[machine] += 1;
        let mut observation = GraphEntityObservation::new(
            enterprise,
            kind,
            AuthorityRef::new(
                adapter,
                format!("tenant-{machine}"),
                format!("{prefix}:{index}"),
            )?,
            BTreeSet::from([format!("service-{index}")]),
            evidence(index, prefix),
        )?;
        observation.critical = kind == EnterpriseEntityKind::Service;
        fixture.member_entities[machine].insert(observation.entity_id.clone());
        if kind == EnterpriseEntityKind::Service {
            if index < 32 {
                fixture
                    .critical_runtime_ids
                    .insert(observation.entity_id.clone());
            }
            if index == 0 {
                fixture.first_runtime_id = Some(observation.entity_id.clone());
            }
        }
        ids.push(observation.entity_id.clone());
        fixture.events[machine].push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance(&machine_id, 1, fixture.sequences[machine]),
            EnterpriseFact::EntityObserved(observation),
        )?);
    }
    for (from, to, kind) in [
        (1, 0, EnterpriseEdgeKind::SourceFor),
        (0, 2, EnterpriseEdgeKind::DeploysTo),
        (0, 3, EnterpriseEdgeKind::AuthenticatesVia),
        (0, 4, EnterpriseEdgeKind::OwnedBy),
        (0, 5, EnterpriseEdgeKind::MonitoredBy),
    ] {
        fixture.sequences[machine] += 1;
        let edge = GraphEdgeObservation::new(
            enterprise,
            ids[from].clone(),
            ids[to].clone(),
            kind,
            None,
            evidence(index, &format!("{kind:?}")),
        )?;
        fixture.member_edges[machine].insert(edge.edge_id.clone());
        fixture.events[machine].push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance(&machine_id, 1, fixture.sequences[machine]),
            EnterpriseFact::EdgeObserved(edge),
        )?);
    }
    fixture.sequences[machine] += 1;
    fixture.events[machine].push(EnterpriseEvent::new(
        enterprise.clone(),
        provenance(&machine_id, 1, fixture.sequences[machine]),
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
            evidence_digests: evidence(index, "simulation"),
        }),
    )?);
    Ok(())
}

fn add_critical_journey(
    enterprise: &EnterpriseId,
    runtime_id: &EnterpriseEntityId,
    events: &mut Vec<EnterpriseEvent>,
    sequence: &mut u64,
    member_entities: &mut BTreeSet<EnterpriseEntityId>,
    member_edges: &mut BTreeSet<EnterpriseEdgeId>,
) -> Result<EnterpriseEntityId, String> {
    let mut ids = Vec::new();
    for (kind, native_id) in [
        (EnterpriseEntityKind::Actor, "actor:customer"),
        (EnterpriseEntityKind::Journey, "journey:checkout"),
        (EnterpriseEntityKind::Database, "database:orders"),
    ] {
        *sequence += 1;
        let mut observation = GraphEntityObservation::new(
            enterprise,
            kind,
            AuthorityRef::new("catalog", "business:acme", native_id)?,
            BTreeSet::from([native_id.into()]),
            evidence(0, native_id),
        )?;
        observation.critical = kind == EnterpriseEntityKind::Journey;
        member_entities.insert(observation.entity_id.clone());
        ids.push(observation.entity_id.clone());
        events.push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance("machine-0", 1, *sequence),
            EnterpriseFact::EntityObserved(observation),
        )?);
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
            evidence(0, &format!("journey:{kind:?}")),
        )?;
        member_edges.insert(edge.edge_id.clone());
        events.push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance("machine-0", 1, *sequence),
            EnterpriseFact::EdgeObserved(edge),
        )?);
    }
    Ok(ids[1].clone())
}

fn add_coverage_pass(
    enterprise: &EnterpriseId,
    epoch: u64,
    member_entities: &[BTreeSet<EnterpriseEntityId>],
    member_edges: &[BTreeSet<EnterpriseEdgeId>],
    events: &mut [Vec<EnterpriseEvent>],
    sequences: &mut [u64],
) -> Result<(), String> {
    for machine in 0..events.len() {
        let machine_id = format!("machine-{machine}");
        let key = coverage_key(machine);
        sequences[machine] += 1;
        let mut coverage = CoverageObservation::new(
            enterprise,
            key.clone(),
            CoverageStatus::Supported,
            None,
            member_entities[machine].len() as u64,
            evidence(machine, &format!("coverage-{epoch}")),
        )?;
        coverage.enumerated_edge_count = member_edges[machine].len() as u64;
        events[machine].push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance(&machine_id, epoch, sequences[machine]),
            EnterpriseFact::CoverageObserved(coverage),
        )?);
        sequences[machine] += 1;
        let mut frontier = FrontierObservation::new(
            enterprise,
            FrontierKey::new(key, None)?,
            FrontierState::Terminal {
                status: CoverageStatus::Supported,
                reason: "authoritative enumerator reached its final cursor".into(),
            },
            evidence(machine, &format!("frontier-{epoch}")),
        )?;
        frontier.discovered_entity_ids = member_entities[machine].clone();
        frontier.discovered_edge_ids = member_edges[machine].clone();
        events[machine].push(EnterpriseEvent::new(
            enterprise.clone(),
            provenance(&machine_id, epoch, sequences[machine]),
            EnterpriseFact::FrontierObserved(frontier),
        )?);
    }
    Ok(())
}

fn coverage_key(machine: usize) -> CoverageKey {
    CoverageKey::new(
        if machine % 2 == 0 { "aws" } else { "gcp" },
        "auth-read-only",
        format!("tenant-{machine}"),
        "all-regions",
        "runtime",
    )
    .expect("valid benchmark coverage key")
}

fn provenance(machine: &str, epoch: u64, sequence: u64) -> EnterpriseProvenance {
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

fn evidence(index: usize, salt: &str) -> BTreeSet<String> {
    BTreeSet::from([format!(
        "{:x}",
        Sha256::digest(format!("{index}:{salt}").as_bytes())
    )])
}
