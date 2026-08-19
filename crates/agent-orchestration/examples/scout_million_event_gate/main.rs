//! Deterministic, offline scale gate for Scout's canonical enterprise graph.
//!
//! The default invocation exercises at least one million real EnterpriseEvent
//! values through canonical batches, EnterpriseGraph replay/materialization,
//! and the public entity-query path. Smaller runs require explicit `--smoke`.

mod fixture;
mod metrics;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use agent_orchestration::{
    EnterpriseBatch, EnterpriseEntityKind, EnterpriseGraph, EnterpriseQuery,
};
use fixture::DeterministicFixture;
use metrics::{serialized_size, write_receipt, RssMetrics};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const MIN_GATE_EVENTS: usize = 1_000_000;
const MAX_EVENTS: usize = 2_000_000;
const MAX_SERVICES: usize = 100_000;

struct Args {
    events: usize,
    services: usize,
    output: PathBuf,
    smoke: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Self {
            events: MIN_GATE_EVENTS,
            services: 10_000,
            output: PathBuf::from("target/scout-benchmark/million-event-gate"),
            smoke: false,
        };
        let mut input = std::env::args().skip(1);
        while let Some(argument) = input.next() {
            match argument.as_str() {
                "--events" => {
                    args.events = parse_count("--events", input.next())?;
                }
                "--services" => {
                    args.services = parse_count("--services", input.next())?;
                }
                "--out" => {
                    args.output = PathBuf::from(input.next().ok_or("--out requires a path")?);
                }
                "--smoke" => args.smoke = true,
                "--help" | "-h" => {
                    println!(
                        "scout_million_event_gate [--events COUNT] [--services COUNT] \
                         [--out PATH] [--smoke]\n\
                         Default: 1,000,000 events and 10,000 services. \
                         --smoke explicitly permits fewer than 1,000,000 events."
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if args.events == 0 || args.events > MAX_EVENTS {
            return Err(format!("events must be in 1..={MAX_EVENTS}"));
        }
        if args.services == 0 || args.services > MAX_SERVICES {
            return Err(format!("services must be in 1..={MAX_SERVICES}"));
        }
        if !args.smoke && args.events < MIN_GATE_EVENTS {
            return Err(format!(
                "the scale gate requires at least {MIN_GATE_EVENTS} events; \
                 use --smoke only for implementation checks"
            ));
        }
        Ok(args)
    }
}

#[derive(Default, Serialize)]
struct PhaseTimings {
    forward_generate_ms: u128,
    forward_measure_storage_ms: u128,
    forward_apply_ms: u128,
    duplicate_apply_ms: u128,
    root_checks_ms: u128,
    forward_materialize_ms: u128,
    forward_snapshot_measure_storage_ms: u128,
    forward_query_ms: u128,
    reverse_generate_ms: u128,
    reverse_apply_ms: u128,
    reverse_materialize_ms: u128,
    reverse_query_ms: u128,
    total_ms: u128,
}

struct IngestResult {
    graph: EnterpriseGraph,
    first_batch: Option<EnterpriseBatch>,
    generated_events: usize,
    serialized_ledger_bytes: u64,
    generate_time: Duration,
    storage_time: Duration,
    apply_time: Duration,
}

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    status: &'static str,
    mode: &'static str,
    events: usize,
    services: usize,
    batches: usize,
    entities: usize,
    edges: usize,
    conflicts: usize,
    event_root: String,
    graph_digest: String,
    query_digest: String,
    target_supporting_events: usize,
    serialized_ledger_bytes: u64,
    serialized_snapshot_bytes: u64,
    replay_order: &'static str,
    duplicate_inserted: usize,
    semantic_sha256: String,
    timings: PhaseTimings,
    memory: RssMetrics,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout million-event gate failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse()?;
    let receipt_path = args.output.join("receipt.json");
    if receipt_path.exists() {
        return Err(format!(
            "refusing to overwrite existing benchmark receipt {}",
            receipt_path.display()
        ));
    }
    let fixture = DeterministicFixture::new(args.events, args.services)?;
    let total_started = Instant::now();
    let mut timings = PhaseTimings::default();
    let mut memory = RssMetrics::new();
    memory.sample("start");

    let mut forward = ingest(&fixture, false, true)?;
    timings.forward_generate_ms = forward.generate_time.as_millis();
    timings.forward_measure_storage_ms = forward.storage_time.as_millis();
    timings.forward_apply_ms = forward.apply_time.as_millis();
    if forward.generated_events != args.events
        || forward.graph.event_count() != args.events
        || forward.graph.batch_count() != fixture.batch_count()
    {
        return Err("forward ingest did not retain every requested event and batch".into());
    }
    memory.sample("forward_ingested");

    let roots_started = Instant::now();
    let root_before_duplicate = forward.graph.event_root()?;
    let duplicate_started = Instant::now();
    let duplicate = forward.graph.apply_batch(
        forward
            .first_batch
            .take()
            .ok_or_else(|| "forward ingest did not retain its first batch".to_string())?,
    )?;
    timings.duplicate_apply_ms = duplicate_started.elapsed().as_millis();
    let root_after_duplicate = forward.graph.event_root()?;
    timings.root_checks_ms = roots_started.elapsed().as_millis();
    if duplicate.inserted != 0
        || duplicate.duplicates != duplicate.received
        || root_before_duplicate != root_after_duplicate
        || forward.graph.event_count() != args.events
    {
        return Err("duplicate replay changed the canonical event set".into());
    }

    let materialize_started = Instant::now();
    let snapshot = forward.graph.snapshot()?;
    timings.forward_materialize_ms = materialize_started.elapsed().as_millis();
    memory.sample("forward_materialized");
    validate_snapshot(&fixture, &snapshot)?;
    if snapshot.event_root != root_after_duplicate {
        return Err("snapshot event root disagrees with the graph event root".into());
    }
    let snapshot_storage_started = Instant::now();
    let serialized_snapshot_bytes = serialized_size(&snapshot)?;
    timings.forward_snapshot_measure_storage_ms = snapshot_storage_started.elapsed().as_millis();
    let event_root = snapshot.event_root.clone();
    let graph_digest = snapshot.graph_digest.clone();
    let entity_count = snapshot.entities.len();
    let edge_count = snapshot.edges.len();
    let conflict_count = snapshot.conflicts.len();
    drop(snapshot);

    let query_started = Instant::now();
    let query = query_target(&fixture, &forward.graph)?;
    timings.forward_query_ms = query_started.elapsed().as_millis();
    let query_digest = digest(&query)?;
    drop(forward.graph);
    memory.sample("forward_dropped");

    let reverse = ingest(&fixture, true, false)?;
    timings.reverse_generate_ms = reverse.generate_time.as_millis();
    timings.reverse_apply_ms = reverse.apply_time.as_millis();
    if reverse.generated_events != args.events
        || reverse.graph.event_count() != args.events
        || reverse.graph.batch_count() != fixture.batch_count()
    {
        return Err("reverse replay did not retain every requested event and batch".into());
    }
    memory.sample("reverse_ingested");

    let reverse_materialize_started = Instant::now();
    let replay_snapshot = reverse.graph.snapshot()?;
    timings.reverse_materialize_ms = reverse_materialize_started.elapsed().as_millis();
    memory.sample("reverse_materialized");
    validate_snapshot(&fixture, &replay_snapshot)?;
    if replay_snapshot.event_root != event_root || replay_snapshot.graph_digest != graph_digest {
        return Err("reverse batch replay changed an enterprise root".into());
    }
    drop(replay_snapshot);
    let reverse_query_started = Instant::now();
    let reverse_query = query_target(&fixture, &reverse.graph)?;
    timings.reverse_query_ms = reverse_query_started.elapsed().as_millis();
    if digest(&reverse_query)? != query_digest || reverse_query != query {
        return Err("reverse batch replay changed public query results".into());
    }
    memory.sample("reverse_queried");

    if !args.smoke && args.events < MIN_GATE_EVENTS {
        return Err("million-event scale gate materialized fewer than one million events".into());
    }
    let semantic = json!({
        "schema": "scout-enterprise-million-event-semantic-v1",
        "events": args.events,
        "services": args.services,
        "batches": fixture.batch_count(),
        "entities": entity_count,
        "edges": edge_count,
        "conflicts": conflict_count,
        "event_root": event_root,
        "graph_digest": graph_digest,
        "query_digest": query_digest,
        "target_supporting_events": fixture.expected_target_supporting_events(),
        "serialized_ledger_bytes": forward.serialized_ledger_bytes,
        "serialized_snapshot_bytes": serialized_snapshot_bytes,
        "duplicate_inserted": duplicate.inserted,
    });
    let semantic_sha256 = digest(&semantic)?;
    timings.total_ms = total_started.elapsed().as_millis();
    memory.sample("complete");
    let receipt = Receipt {
        schema: "scout-enterprise-million-event-gate-v1",
        status: if args.smoke { "smoke_passed" } else { "passed" },
        mode: if args.smoke { "smoke" } else { "scale_gate" },
        events: args.events,
        services: args.services,
        batches: fixture.batch_count(),
        entities: entity_count,
        edges: edge_count,
        conflicts: conflict_count,
        event_root,
        graph_digest,
        query_digest,
        target_supporting_events: fixture.expected_target_supporting_events(),
        serialized_ledger_bytes: forward.serialized_ledger_bytes,
        serialized_snapshot_bytes,
        replay_order: "reverse_batches",
        duplicate_inserted: duplicate.inserted,
        semantic_sha256,
        timings,
        memory,
    };
    write_receipt(&receipt_path, &receipt)?;
    println!("receipt={}", receipt_path.display());
    println!("status={}", receipt.status);
    println!("events={}", receipt.events);
    println!("event_root={}", receipt.event_root);
    println!("graph_digest={}", receipt.graph_digest);
    println!("semantic_sha256={}", receipt.semantic_sha256);
    Ok(())
}

fn ingest(
    fixture: &DeterministicFixture,
    reverse: bool,
    measure_storage: bool,
) -> Result<IngestResult, String> {
    let mut graph = EnterpriseGraph::new(fixture.enterprise_id().clone());
    let mut first_batch = None;
    let mut generated_events = 0;
    let mut serialized_ledger_bytes = 0_u64;
    let mut generate_time = Duration::ZERO;
    let mut storage_time = Duration::ZERO;
    let mut apply_time = Duration::ZERO;
    for offset in 0..fixture.batch_count() {
        let batch_index = if reverse {
            fixture.batch_count() - 1 - offset
        } else {
            offset
        };
        let generate_started = Instant::now();
        let batch = fixture.batch(batch_index)?;
        generate_time += generate_started.elapsed();
        generated_events += batch.events.len();
        if !reverse && batch_index == 0 {
            first_batch = Some(batch.clone());
        }
        if measure_storage {
            let storage_started = Instant::now();
            serialized_ledger_bytes = serialized_ledger_bytes
                .checked_add(serialized_size(&batch)?)
                .ok_or_else(|| "serialized ledger byte count overflow".to_string())?;
            storage_time += storage_started.elapsed();
        }
        let apply_started = Instant::now();
        let report = graph.apply_batch(batch)?;
        apply_time += apply_started.elapsed();
        if report.inserted + report.duplicates != report.received || report.duplicates != 0 {
            return Err(format!(
                "batch {batch_index} did not insert its canonical event set exactly once"
            ));
        }
    }
    Ok(IngestResult {
        graph,
        first_batch,
        generated_events,
        serialized_ledger_bytes,
        generate_time,
        storage_time,
        apply_time,
    })
}

fn validate_snapshot(
    fixture: &DeterministicFixture,
    snapshot: &agent_orchestration::EnterpriseSnapshot,
) -> Result<(), String> {
    if snapshot.event_count != fixture.event_count()
        || snapshot.entities.len() != fixture.service_count() * 2
        || snapshot.edges.len() != fixture.service_count()
        || snapshot.retracted_event_count != 0
        || !snapshot.conflicts.is_empty()
    {
        return Err("materialized snapshot has unexpected enterprise counts or conflicts".into());
    }
    let target = fixture.target_service_id()?;
    let supporting_events = snapshot
        .entities
        .get(&target)
        .ok_or_else(|| "target service was not materialized".to_string())?
        .supporting_events
        .len();
    if supporting_events != fixture.expected_target_supporting_events() {
        return Err(format!(
            "target service has {supporting_events} supporting events; expected {}",
            fixture.expected_target_supporting_events()
        ));
    }
    Ok(())
}

fn query_target(
    fixture: &DeterministicFixture,
    graph: &EnterpriseGraph,
) -> Result<Vec<agent_orchestration::MaterializedEntity>, String> {
    let result = graph.query_entities(&EnterpriseQuery {
        kind: Some(EnterpriseEntityKind::Service),
        provider_namespace: Some("benchmark".into()),
        authority_scope: Some("tenant-scale".into()),
        label_contains: Some(fixture.target_label()),
        critical: Some(fixture.target_service_index().is_multiple_of(100)),
        after_entity_id: None,
        limit: 10,
    })?;
    if result.len() != 1
        || result[0].entity_id != fixture.target_service_id()?
        || result[0].supporting_events.len() != fixture.expected_target_supporting_events()
    {
        return Err("public enterprise entity query returned the wrong target service".into());
    }
    Ok(result)
}

fn digest(value: &impl Serialize) -> Result<String, String> {
    let body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(body)))
}

fn parse_count(flag: &str, value: Option<String>) -> Result<usize, String> {
    value
        .ok_or_else(|| format!("{flag} requires a count"))?
        .parse()
        .map_err(|_| format!("{flag} must be an integer"))
}
