//! Repeatable scale gate for Scout's affected-key enterprise projection.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGraph, EnterpriseId, EnterpriseProvenance, GraphEntityObservation,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const EVENTS_PER_BATCH: usize = 10_000;

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    status: &'static str,
    host_label: String,
    baseline_events: usize,
    baseline_entities: usize,
    update_events: usize,
    full_duration_ns: u128,
    affected_duration_ns: u128,
    duration_speedup: f64,
    full_candidate_events: usize,
    affected_candidate_events: usize,
    candidate_reduction: f64,
    full_projection_rows: usize,
    affected_projection_rows: usize,
    row_reduction: f64,
    graph_digest: String,
    semantic_digest: String,
}

struct Args {
    events: usize,
    entities: usize,
    host_label: String,
    out: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    if args.events < args.entities || args.events % args.entities != 0 {
        return Err("events must be an exact multiple of entities".into());
    }
    let enterprise = EnterpriseId::new("affected-projection-benchmark")?;
    let mut baseline = EnterpriseGraph::new(enterprise.clone());
    for first in (0..args.events).step_by(EVENTS_PER_BATCH) {
        let end = (first + EVENTS_PER_BATCH).min(args.events);
        let events = (first..end)
            .map(|event_index| {
                let entity_index = event_index % args.entities;
                entity_event(&enterprise, entity_index, 1, event_index + 1, "baseline")
            })
            .collect::<Result<Vec<_>, _>>()?;
        baseline.apply_batch(EnterpriseBatch::new(enterprise.clone(), events)?)?;
    }
    let baseline_snapshot = baseline.snapshot()?;
    if baseline_snapshot.event_count != args.events
        || baseline_snapshot.entities.len() != args.entities
    {
        return Err("baseline fixture did not materialize requested scale".into());
    }

    let target = args.entities / 2;
    let update = EnterpriseBatch::new(
        enterprise.clone(),
        [entity_event(
            &enterprise,
            target,
            2,
            args.events + 1,
            "updated",
        )?],
    )?;

    let mut full_graph = baseline.clone();
    let full_started = Instant::now();
    full_graph.apply_batch(update.clone())?;
    let full_snapshot = full_graph.snapshot()?;
    let full_duration = full_started.elapsed();

    let mut affected_graph = baseline;
    let mut cursor = affected_graph.projection_cursor()?;
    let affected_started = Instant::now();
    let affected = affected_graph.apply_batch_affected(&mut cursor, update)?;
    let affected_duration = affected_started.elapsed();
    if affected.requires_full_rebuild() || affected.affected_row_count() != 1 {
        return Err("one entity update did not stay on the affected-key path".into());
    }
    let affected_entity = affected
        .entities
        .values()
        .next()
        .and_then(Option::as_ref)
        .ok_or("affected projection did not return its entity row")?;
    if full_snapshot.entities.get(&affected_entity.entity_id) != Some(affected_entity) {
        return Err("affected entity row differs from full materialization".into());
    }
    let replay_snapshot = affected_graph.snapshot()?;
    if replay_snapshot != full_snapshot {
        return Err("affected update changed deterministic full replay".into());
    }

    let full_candidates = full_snapshot.event_count;
    let affected_candidates = affected.work.candidate_events_examined;
    let full_rows = full_snapshot.entities.len()
        + full_snapshot.edges.len()
        + full_snapshot.coverage.len()
        + full_snapshot.frontier.len()
        + full_snapshot.simulation_contracts.len();
    let affected_rows = affected.affected_row_count();
    let candidate_reduction = ratio(full_candidates, affected_candidates);
    let row_reduction = ratio(full_rows, affected_rows);
    if candidate_reduction < 1_000.0 || row_reduction < 1_000.0 {
        return Err("affected-key work reduction missed the 1000x scale gate".into());
    }

    let semantic_digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &full_snapshot.graph_digest,
            &affected_entity.entity_id,
            affected_entity,
        ))?)
    );
    let receipt = Receipt {
        schema: "scout-affected-projection-gate-v1",
        status: "passed",
        host_label: args.host_label,
        baseline_events: args.events,
        baseline_entities: args.entities,
        update_events: affected.work.inserted_events,
        full_duration_ns: full_duration.as_nanos(),
        affected_duration_ns: affected_duration.as_nanos(),
        duration_speedup: full_duration.as_secs_f64() / affected_duration.as_secs_f64(),
        full_candidate_events: full_candidates,
        affected_candidate_events: affected_candidates,
        candidate_reduction,
        full_projection_rows: full_rows,
        affected_projection_rows: affected_rows,
        row_reduction,
        graph_digest: full_snapshot.graph_digest,
        semantic_digest,
    };
    fs::create_dir_all(&args.out)?;
    let receipt_path = args.out.join("receipt.json");
    fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt)?)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    println!("receipt={}", receipt_path.display());
    Ok(())
}

fn entity_event(
    enterprise: &EnterpriseId,
    entity_index: usize,
    epoch: u64,
    sequence: usize,
    label: &str,
) -> Result<EnterpriseEvent, String> {
    let observation = GraphEntityObservation::new(
        enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new(
            "benchmark",
            "tenant-scale",
            format!("service:{entity_index:08}"),
        )?,
        BTreeSet::from([format!("{label}-{entity_index:08}")]),
        BTreeSet::from([format!(
            "{:x}",
            Sha256::digest(format!("affected/{label}/{entity_index}").as_bytes())
        )]),
    )?;
    EnterpriseEvent::new(
        enterprise.clone(),
        EnterpriseProvenance {
            machine_id: "benchmark-machine".into(),
            run_id: format!("affected-projection-epoch-{epoch}"),
            adapter_instance_id: "benchmark-adapter".into(),
            auth_context_id: "benchmark-read-only".into(),
            discovery_epoch: format!("epoch-{epoch}"),
            discovery_epoch_sequence: epoch,
            source_sequence: u64::try_from(sequence)
                .map_err(|_| "benchmark sequence does not fit in u64".to_string())?,
            observed_at_ms: 1_700_000_000_000
                + u64::try_from(sequence)
                    .map_err(|_| "benchmark timestamp does not fit in u64".to_string())?,
            source_fingerprint: "5".repeat(64),
        },
        EnterpriseFact::EntityObserved(observation),
    )
}

fn parse_args() -> Result<Args, String> {
    let mut events = 100_000;
    let mut entities = 10_000;
    let mut host_label = "local".to_string();
    let mut out = PathBuf::from("target/scout-benchmark/affected-projection-100k-local-v1");
    let mut values = env::args().skip(1);
    while let Some(flag) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--events" => events = value.parse().map_err(|_| "invalid --events")?,
            "--entities" => entities = value.parse().map_err(|_| "invalid --entities")?,
            "--host-label" => host_label = value,
            "--out" => out = PathBuf::from(value),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(Args {
        events,
        entities,
        host_label,
        out,
    })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    numerator as f64 / denominator.max(1) as f64
}
