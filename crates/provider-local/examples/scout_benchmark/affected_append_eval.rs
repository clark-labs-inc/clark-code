use std::time::Instant;

use agent_orchestration::EnterpriseFact;
use scout_store::{IndexReceipt, ScoutStoreRequest, ScoutStoreResponse};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "affected_append_eval/fixture.rs"]
mod fixture;
use fixture::ScaleFixture;

const MIN_SCALE_CEILING: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct StructuralCounters {
    events_replayed: usize,
    event_ids_scanned: usize,
    entity_rows_read: usize,
    edge_rows_read: usize,
    history_rows_read: usize,
    auxiliary_rows_read: usize,
    conflict_rows_read: usize,
    conflict_rows_written: usize,
    conflict_rows_deleted: usize,
    incident_edges_reclassified: usize,
    affected_projection_rows: usize,
    full_projection_fallback: bool,
}

impl From<&IndexReceipt> for StructuralCounters {
    fn from(receipt: &IndexReceipt) -> Self {
        Self {
            events_replayed: receipt.events_replayed,
            event_ids_scanned: receipt.event_ids_scanned,
            entity_rows_read: receipt.entity_rows_read,
            edge_rows_read: receipt.edge_rows_read,
            history_rows_read: receipt.history_rows_read,
            auxiliary_rows_read: receipt.auxiliary_rows_read,
            conflict_rows_read: receipt.conflict_rows_read,
            conflict_rows_written: receipt.conflict_rows_written,
            conflict_rows_deleted: receipt.conflict_rows_deleted,
            incident_edges_reclassified: receipt.incident_edges_reclassified,
            affected_projection_rows: receipt.affected_projection_rows,
            full_projection_fallback: receipt.full_projection_fallback,
        }
    }
}

#[derive(Debug, Serialize)]
struct ScaleSample {
    services: usize,
    seed_events: usize,
    seed_edges: usize,
    hot_append_wall_ms: u128,
    counters: StructuralCounters,
    event_root: String,
    graph_digest: String,
    event_set_root_v1: String,
    projection_map_root_v2: String,
    enterprise_snapshot_root_v2: String,
    status_sha256: String,
    affected_state_sha256: String,
}

pub fn affected_row_append_scaling(requested_services: usize) -> Result<(String, Value), String> {
    let sizes = scale_sizes(requested_services);
    let mut samples = Vec::with_capacity(sizes.len());
    for size in sizes {
        samples.push(run_sample(size)?);
    }

    let expected = samples
        .first()
        .ok_or_else(|| "affected-row benchmark did not select any scales".to_string())?
        .counters
        .clone();
    validate_structural_signature(&expected)?;
    for sample in &samples[1..] {
        if sample.counters != expected {
            return Err(format!(
                "affected-row work grew with graph size: {} services produced {:?}, expected {:?}",
                sample.services, sample.counters, expected
            ));
        }
    }

    let min_wall_ms = samples
        .iter()
        .map(|sample| sample.hot_append_wall_ms)
        .min()
        .unwrap_or_default();
    let max_wall_ms = samples
        .iter()
        .map(|sample| sample.hot_append_wall_ms)
        .max()
        .unwrap_or_default();
    let semantic_sha256 = digest(&samples.iter().map(semantic_sample).collect::<Vec<_>>())?;

    Ok((
        format!(
            "constant-degree append touched the same bounded rows at {} graph scales",
            samples.len()
        ),
        json!({
            "requested_services": requested_services,
            "minimum_scale_ceiling_services": MIN_SCALE_CEILING,
            "scales": samples,
            "scale_invariant_structural_signature": expected,
            "min_hot_append_wall_ms": min_wall_ms,
            "max_hot_append_wall_ms": max_wall_ms,
            "wall_time_ratio_with_one_ms_floor":
                max_wall_ms as f64 / min_wall_ms.max(1) as f64,
            "semantic_sha256": semantic_sha256,
        }),
    ))
}

fn scale_sizes(requested_services: usize) -> Vec<usize> {
    let ceiling = requested_services.max(MIN_SCALE_CEILING);
    let middle = (ceiling / 4).clamp(128, 1_024);
    let mut sizes = vec![64, middle, ceiling];
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn validate_structural_signature(counters: &StructuralCounters) -> Result<(), String> {
    if counters.full_projection_fallback {
        return Err("ordinary affected-row append used the full projection fallback".into());
    }
    if counters.events_replayed != 1 {
        return Err(format!(
            "one entity reobservation replayed {} cached events instead of 1",
            counters.events_replayed
        ));
    }
    if counters.event_ids_scanned != 0
        || counters.history_rows_read != 0
        || counters.auxiliary_rows_read != 0
        || counters.conflict_rows_read != 0
        || counters.conflict_rows_written != 0
        || counters.conflict_rows_deleted != 0
    {
        return Err(format!(
            "ordinary append scanned global state: {counters:?}"
        ));
    }
    if counters.affected_projection_rows != 1 {
        return Err(format!(
            "one entity update affected {} direct projection rows",
            counters.affected_projection_rows
        ));
    }
    if counters.incident_edges_reclassified != 2 {
        return Err(format!(
            "constant-degree entity update reclassified {} edges instead of 2",
            counters.incident_edges_reclassified
        ));
    }
    if counters.entity_rows_read > 3 || counters.edge_rows_read > 2 {
        return Err(format!(
            "ordinary append exceeded its constant row-read budget: {counters:?}"
        ));
    }
    Ok(())
}

fn run_sample(services: usize) -> Result<ScaleSample, String> {
    let fixture = ScaleFixture::new(services)?;
    let (initial_receipt, seed_events) = fixture.rebuild()?;
    if !initial_receipt.rebuilt {
        return Err("affected-row scale seed did not take the cold rebuild path".into());
    }

    let update = fixture.updated_middle_entity()?;
    let updated_entity_id = update.entity_id.clone();
    let envelope = fixture.sign_facts(
        "affected-row-hot-append",
        2,
        vec![EnterpriseFact::EntityObserved(update)],
    )?;
    let started = Instant::now();
    let response = fixture.call(ScoutStoreRequest::Ingest {
        enterprise_id: fixture.enterprise.clone(),
        envelope: Box::new(envelope),
    })?;
    let hot_append_wall_ms = started.elapsed().as_millis();
    let ScoutStoreResponse::Ingested {
        receipt: hot_receipt,
        ..
    } = response
    else {
        return Err("affected-row append returned the wrong response".into());
    };
    if hot_receipt.rebuilt
        || hot_receipt.ledger_authority_work.envelope_rows_read != 0
        || hot_receipt.derived_batches_read != 0
    {
        return Err(format!(
            "affected-row append did not remain on the authenticated hot path: {hot_receipt:?}"
        ));
    }

    let hot_status = fixture.status()?;
    let hot_state = fixture.affected_state(&updated_entity_id)?;
    let cold_receipt = fixture.force_cold()?;
    let cold_status = fixture.status()?;
    let cold_state = fixture.affected_state(&updated_entity_id)?;
    if hot_status != cold_status {
        return Err(format!(
            "hot and cold status diverged at {services} services"
        ));
    }
    if hot_state != cold_state {
        return Err(format!(
            "hot and cold affected rows diverged at {services} services"
        ));
    }
    assert_roots_equal(&hot_receipt, &cold_receipt, services)?;

    Ok(ScaleSample {
        services,
        seed_events,
        seed_edges: services.saturating_sub(1),
        hot_append_wall_ms,
        counters: StructuralCounters::from(&hot_receipt),
        event_root: hot_receipt.event_root,
        graph_digest: hot_receipt.graph_digest,
        event_set_root_v1: required_root(hot_receipt.event_set_root_v1, "event-set", services)?,
        projection_map_root_v2: required_root(
            hot_receipt.projection_map_root_v2,
            "projection-map",
            services,
        )?,
        enterprise_snapshot_root_v2: required_root(
            hot_receipt.enterprise_snapshot_root_v2,
            "enterprise-snapshot",
            services,
        )?,
        status_sha256: digest(&hot_status)?,
        affected_state_sha256: digest(&hot_state)?,
    })
}

fn semantic_sample(sample: &ScaleSample) -> Value {
    json!({
        "services": sample.services,
        "seed_events": sample.seed_events,
        "seed_edges": sample.seed_edges,
        "counters": sample.counters,
        "event_root": sample.event_root,
        "graph_digest": sample.graph_digest,
        "event_set_root_v1": sample.event_set_root_v1,
        "projection_map_root_v2": sample.projection_map_root_v2,
        "enterprise_snapshot_root_v2": sample.enterprise_snapshot_root_v2,
        "status_sha256": sample.status_sha256,
        "affected_state_sha256": sample.affected_state_sha256,
    })
}

fn required_root(root: Option<String>, kind: &str, services: usize) -> Result<String, String> {
    root.ok_or_else(|| {
        format!("affected-row hot receipt omitted {kind} root at {services} services")
    })
}

fn assert_roots_equal(
    hot: &IndexReceipt,
    cold: &IndexReceipt,
    services: usize,
) -> Result<(), String> {
    if hot.event_root != cold.event_root
        || hot.graph_digest != cold.graph_digest
        || hot.event_set_root_v1 != cold.event_set_root_v1
        || hot.projection_map_root_v2 != cold.projection_map_root_v2
        || hot.enterprise_snapshot_root_v2 != cold.enterprise_snapshot_root_v2
    {
        return Err(format!(
            "hot and cold authenticated roots diverged at {services} services"
        ));
    }
    Ok(())
}

fn digest(value: &impl Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(to_string)?)
    ))
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
