use std::time::Instant;

use scout_store::{IndexReceipt, ScoutStoreRequest, ScoutStoreResponse};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "conflict_append_eval/fixture.rs"]
mod fixture;
use fixture::ConflictFixture;

const PREVIEW_LIMIT: usize = 64;

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
struct ConflictScaleSample {
    conflicts: usize,
    seed_events: usize,
    normalized_conflict_rows: usize,
    hot_append_wall_ms: u128,
    projection_rows_per_wall_ms_with_one_ms_floor: f64,
    counters: StructuralCounters,
    event_root: String,
    graph_digest: String,
    event_set_root_v1: String,
    projection_map_root_v2: String,
    enterprise_snapshot_root_v2: String,
    status_sha256: String,
}

pub fn conflict_append_scaling(requested_conflicts: usize) -> Result<(String, Value), String> {
    let sizes = scale_sizes(requested_conflicts);
    let mut samples = Vec::with_capacity(sizes.len());
    for size in sizes {
        samples.push(run_sample(size)?);
    }
    let expected = samples
        .first()
        .ok_or_else(|| "conflict-corpus benchmark selected no scales".to_string())?
        .counters
        .clone();
    validate_structural_signature(&expected)?;
    for sample in &samples[1..] {
        if sample.counters != expected {
            return Err(format!(
                "unrelated append conflict work grew with corpus size: {} conflicts produced {:?}, expected {:?}",
                sample.conflicts, sample.counters, expected
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
    let first_p_over_wall = samples
        .first()
        .map(|sample| sample.projection_rows_per_wall_ms_with_one_ms_floor)
        .unwrap_or_default();
    let last_p_over_wall = samples
        .last()
        .map(|sample| sample.projection_rows_per_wall_ms_with_one_ms_floor)
        .unwrap_or_default();
    let semantic_sha256 = digest(&samples.iter().map(semantic_sample).collect::<Vec<_>>())?;

    Ok((
        format!(
            "unrelated append kept normalized conflict work constant across {} corpus scales",
            samples.len()
        ),
        json!({
            "requested_conflicts": requested_conflicts,
            "scales": samples,
            "scale_invariant_structural_signature": expected,
            "min_hot_append_wall_ms": min_wall_ms,
            "max_hot_append_wall_ms": max_wall_ms,
            "wall_time_ratio_with_one_ms_floor":
                max_wall_ms as f64 / min_wall_ms.max(1) as f64,
            "projection_rows_per_wall_ratio_ceiling_to_64_with_one_ms_floor":
                ratio(last_p_over_wall, first_p_over_wall),
            "wall_plateau_enforced": false,
            "wall_plateau_reason": "ledger authority integration remains in progress",
            "semantic_sha256": semantic_sha256,
        }),
    ))
}

fn scale_sizes(requested_conflicts: usize) -> Vec<usize> {
    let ceiling = requested_conflicts.max(PREVIEW_LIMIT);
    let middle = ((PREVIEW_LIMIT + ceiling) / 2).clamp(PREVIEW_LIMIT, ceiling);
    let mut sizes = vec![PREVIEW_LIMIT, middle, ceiling];
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn run_sample(conflicts: usize) -> Result<ConflictScaleSample, String> {
    let fixture = ConflictFixture::new(conflicts)?;
    let initial = fixture.rebuild()?;
    if !initial.rebuilt {
        return Err("conflict corpus did not take the cold seed path".into());
    }
    let seeded_status = fixture.status()?;
    validate_status(&seeded_status, conflicts)?;
    assert_normalized_count(&fixture, conflicts, "seed")?;

    let envelope = fixture.unrelated_entity_envelope()?;
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
        return Err("conflict-corpus append returned the wrong response".into());
    };
    if hot_receipt.rebuilt
        || hot_receipt.ledger_authority_work.envelope_rows_read != 0
        || hot_receipt.derived_batches_read != 0
    {
        return Err(format!(
            "conflict-corpus append missed the authenticated hot path: {hot_receipt:?}"
        ));
    }
    let counters = StructuralCounters::from(&hot_receipt);
    validate_structural_signature(&counters)?;

    let hot_status = fixture.status()?;
    validate_status(&hot_status, conflicts)?;
    assert_normalized_count(&fixture, conflicts, "hot append")?;
    let cold_receipt = fixture.force_cold()?;
    let cold_status = fixture.status()?;
    assert_normalized_count(&fixture, conflicts, "cold rebuild")?;
    if hot_status != cold_status {
        return Err(format!(
            "hot and cold status diverged at {conflicts} conflicts"
        ));
    }
    assert_roots_equal(&hot_receipt, &cold_receipt, conflicts)?;

    Ok(ConflictScaleSample {
        conflicts,
        seed_events: conflicts * 2,
        normalized_conflict_rows: conflicts,
        hot_append_wall_ms,
        projection_rows_per_wall_ms_with_one_ms_floor: counters.affected_projection_rows as f64
            / hot_append_wall_ms.max(1) as f64,
        counters,
        event_root: hot_receipt.event_root,
        graph_digest: hot_receipt.graph_digest,
        event_set_root_v1: required_root(hot_receipt.event_set_root_v1, "event-set", conflicts)?,
        projection_map_root_v2: required_root(
            hot_receipt.projection_map_root_v2,
            "projection-map",
            conflicts,
        )?,
        enterprise_snapshot_root_v2: required_root(
            hot_receipt.enterprise_snapshot_root_v2,
            "enterprise-snapshot",
            conflicts,
        )?,
        status_sha256: digest(&hot_status)?,
    })
}

fn validate_structural_signature(counters: &StructuralCounters) -> Result<(), String> {
    if counters.full_projection_fallback
        || counters.events_replayed != 0
        || counters.event_ids_scanned != 0
        || counters.entity_rows_read != 0
        || counters.edge_rows_read != 0
        || counters.history_rows_read != 0
        || counters.auxiliary_rows_read != 0
        || counters.conflict_rows_read != PREVIEW_LIMIT
        || counters.conflict_rows_written != 0
        || counters.conflict_rows_deleted != 0
        || counters.incident_edges_reclassified != 0
        || counters.affected_projection_rows != 1
    {
        return Err(format!(
            "unrelated append exceeded its constant conflict-work budget: {counters:?}"
        ));
    }
    Ok(())
}

fn validate_status(
    status: &scout_store::IndexedStatus,
    expected_conflicts: usize,
) -> Result<(), String> {
    if status.conflict_count != expected_conflicts || status.conflicts.len() != PREVIEW_LIMIT {
        return Err(format!(
            "normalized conflict status mismatch: expected {expected_conflicts} with {PREVIEW_LIMIT}-row preview, got {} with {}",
            status.conflict_count,
            status.conflicts.len()
        ));
    }
    Ok(())
}

fn semantic_sample(sample: &ConflictScaleSample) -> Value {
    json!({
        "conflicts": sample.conflicts,
        "seed_events": sample.seed_events,
        "normalized_conflict_rows": sample.normalized_conflict_rows,
        "counters": sample.counters,
        "event_root": sample.event_root,
        "graph_digest": sample.graph_digest,
        "event_set_root_v1": sample.event_set_root_v1,
        "projection_map_root_v2": sample.projection_map_root_v2,
        "enterprise_snapshot_root_v2": sample.enterprise_snapshot_root_v2,
        "status_sha256": sample.status_sha256,
    })
}

fn assert_normalized_count(
    fixture: &ConflictFixture,
    expected: usize,
    phase: &str,
) -> Result<(), String> {
    let observed = fixture.normalized_conflict_count()?;
    if observed != expected {
        return Err(format!(
            "{phase} retained {observed} normalized conflict rows instead of {expected}"
        ));
    }
    Ok(())
}

fn required_root(root: Option<String>, kind: &str, conflicts: usize) -> Result<String, String> {
    root.ok_or_else(|| {
        format!("conflict-corpus hot receipt omitted {kind} root at {conflicts} conflicts")
    })
}

fn assert_roots_equal(
    hot: &IndexReceipt,
    cold: &IndexReceipt,
    conflicts: usize,
) -> Result<(), String> {
    if hot.event_root != cold.event_root
        || hot.graph_digest != cold.graph_digest
        || hot.event_set_root_v1 != cold.event_set_root_v1
        || hot.projection_map_root_v2 != cold.projection_map_root_v2
        || hot.enterprise_snapshot_root_v2 != cold.enterprise_snapshot_root_v2
    {
        return Err(format!(
            "hot and cold authenticated roots diverged at {conflicts} conflicts"
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

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
