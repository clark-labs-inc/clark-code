use std::time::Instant;

use scout_store::IndexReceipt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "high_fan_in_eval/fixture.rs"]
mod fixture;
#[path = "high_fan_in_eval/metrics.rs"]
mod metrics;

use fixture::{FanInFixture, LOCATOR_KINDS};
use metrics::RssMetrics;

const SWEEP_SIZES: [usize; 3] = [1_000, 10_000, 100_000];
const MAX_FIXED_WIDTH_DELTA_BYTES: usize = 16;

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
struct BaselineExpectations {
    events_replayed_equals_zero: bool,
    fixed_width_materialized_row: bool,
    fixed_width_max_delta_bytes: usize,
}

#[derive(Debug, Serialize)]
struct AppendSample {
    locator_kind: &'static str,
    wall_ms: u128,
    row_bytes_before: usize,
    row_bytes_after: usize,
    row_growth_bytes: i64,
    row_sha256_after: String,
    counters: StructuralCounters,
    expectations: BaselineExpectations,
    baseline_failures: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ScaleSample {
    observations_per_locator_before_append: usize,
    seed_events: usize,
    seed_write_wall_ms: u128,
    initial_rebuild_wall_ms: u128,
    hot_path_prime_wall_ms: u128,
    hot_path_prime_counters: StructuralCounters,
    cold_comparison_wall_ms: u128,
    total_wall_ms: u128,
    database_bytes_after_seed: u64,
    database_bytes_after_hot_appends: u64,
    database_bytes_after_cold_comparison: u64,
    appends: Vec<AppendSample>,
    rss: RssMetrics,
    hot_cold_status_equal: bool,
    hot_cold_rows_equal: bool,
    hot_cold_roots_equal: bool,
    status_sha256: String,
    rows_sha256: String,
    event_root: String,
    graph_digest: String,
    event_set_root_v1: String,
    projection_map_root_v2: String,
    enterprise_snapshot_root_v2: String,
}

pub(super) fn high_fan_in_baseline(
    requested_n: usize,
    sweep: bool,
) -> Result<(String, Value), String> {
    let sizes = selected_sizes(requested_n, sweep);
    let mut samples = Vec::with_capacity(sizes.len());
    for n in sizes {
        samples.push(run_sample(n)?);
    }
    let failure_count = samples
        .iter()
        .flat_map(|sample| &sample.appends)
        .map(|append| append.baseline_failures.len())
        .sum::<usize>();
    let semantic_sha256 = digest(
        &samples
            .iter()
            .map(semantic_sample)
            .collect::<Vec<serde_json::Value>>(),
    )?;
    Ok((
        format!(
            "recorded {} same-locator append baselines; {} expected future-gate failures observed",
            samples.len() * LOCATOR_KINDS.len(),
            failure_count
        ),
        json!({
            "mode": if sweep { "sweep_1k_10k_100k" } else { "single" },
            "requested_observations_per_locator": requested_n,
            "locator_kinds": LOCATOR_KINDS,
            "expectations_are_observational_not_case_gates": true,
            "samples": samples,
            "baseline_failure_count": failure_count,
            "semantic_sha256": semantic_sha256,
        }),
    ))
}

fn selected_sizes(requested_n: usize, sweep: bool) -> Vec<usize> {
    if sweep {
        SWEEP_SIZES.to_vec()
    } else {
        vec![requested_n]
    }
}

fn run_sample(n: usize) -> Result<ScaleSample, String> {
    let total = Instant::now();
    let mut rss = RssMetrics::new();
    rss.sample("start");
    let fixture = FanInFixture::new(n)?;

    let started = Instant::now();
    let seed_events = fixture.write_seed_batches()?;
    let seed_write_wall_ms = started.elapsed().as_millis();
    rss.sample("seed_batches_written");

    let started = Instant::now();
    let initial_receipt = fixture.rebuild()?;
    let initial_rebuild_wall_ms = started.elapsed().as_millis();
    if !initial_receipt.rebuilt || initial_receipt.events_replayed != seed_events {
        return Err(format!(
            "high-fan-in initial rebuild receipt was inconsistent at N={n}: {initial_receipt:?}"
        ));
    }
    rss.sample("initial_rebuild");
    let database_bytes_after_seed = fixture.database_bytes()?;
    let mut rows = fixture.rows()?;
    let started = Instant::now();
    let prime_receipt = fixture.prime_hot_path()?;
    let hot_path_prime_wall_ms = started.elapsed().as_millis();
    if prime_receipt.rebuilt
        || prime_receipt.ledger_authority_work.envelope_rows_read != 0
        || prime_receipt.derived_batches_read != 0
    {
        return Err(format!(
            "high-fan-in prime left the hot path at N={n}: {prime_receipt:?}"
        ));
    }
    let hot_path_prime_counters = StructuralCounters::from(&prime_receipt);
    rss.sample("hot_path_prime");
    let mut appends = Vec::with_capacity(LOCATOR_KINDS.len());
    let mut final_hot_receipt = None;

    for kind in LOCATOR_KINDS {
        let row_bytes_before = rows.byte_len(kind)?;
        let started = Instant::now();
        let receipt = fixture.append(kind)?;
        let wall_ms = started.elapsed().as_millis();
        if receipt.rebuilt
            || receipt.ledger_authority_work.envelope_rows_read != 0
            || receipt.derived_batches_read != 0
        {
            return Err(format!(
                "{kind} high-fan-in append left the hot path at N={n}: {receipt:?}"
            ));
        }
        let next_rows = fixture.rows()?;
        let row_bytes_after = next_rows.byte_len(kind)?;
        let counters = StructuralCounters::from(&receipt);
        let row_delta = row_bytes_after.abs_diff(row_bytes_before);
        let expectations = BaselineExpectations {
            events_replayed_equals_zero: counters.events_replayed == 0,
            fixed_width_materialized_row: row_delta <= MAX_FIXED_WIDTH_DELTA_BYTES,
            fixed_width_max_delta_bytes: MAX_FIXED_WIDTH_DELTA_BYTES,
        };
        let mut failures = Vec::new();
        if !expectations.events_replayed_equals_zero {
            failures.push("events_replayed==0");
        }
        if !expectations.fixed_width_materialized_row {
            failures.push("fixed_width_materialized_row");
        }
        appends.push(AppendSample {
            locator_kind: kind,
            wall_ms,
            row_bytes_before,
            row_bytes_after,
            row_growth_bytes: signed_delta(row_bytes_before, row_bytes_after)?,
            row_sha256_after: digest(
                next_rows
                    .json_by_kind
                    .get(kind)
                    .ok_or_else(|| format!("missing {kind} materialized row after append"))?,
            )?,
            counters,
            expectations,
            baseline_failures: failures,
        });
        rows = next_rows;
        final_hot_receipt = Some(receipt);
        rss.sample(&format!("hot_append_{kind}"));
    }

    let hot_receipt =
        final_hot_receipt.ok_or_else(|| "high-fan-in case ran no appends".to_string())?;
    let hot_status = fixture.status()?;
    let hot_rows = rows;
    let database_bytes_after_hot_appends = fixture.database_bytes()?;
    let started = Instant::now();
    let cold_receipt = fixture.force_cold()?;
    let cold_comparison_wall_ms = started.elapsed().as_millis();
    let cold_status = fixture.status()?;
    let cold_rows = fixture.rows()?;
    rss.sample("cold_comparison");

    let hot_cold_status_equal = hot_status == cold_status;
    let hot_cold_rows_equal = hot_rows == cold_rows;
    let hot_cold_roots_equal = roots_equal(&hot_receipt, &cold_receipt);
    if !(hot_cold_status_equal && hot_cold_rows_equal && hot_cold_roots_equal) {
        return Err(format!("high-fan-in hot/cold equality failed at N={n}"));
    }

    Ok(ScaleSample {
        observations_per_locator_before_append: n,
        seed_events,
        seed_write_wall_ms,
        initial_rebuild_wall_ms,
        hot_path_prime_wall_ms,
        hot_path_prime_counters,
        cold_comparison_wall_ms,
        total_wall_ms: total.elapsed().as_millis(),
        database_bytes_after_seed,
        database_bytes_after_hot_appends,
        database_bytes_after_cold_comparison: fixture.database_bytes()?,
        appends,
        rss,
        hot_cold_status_equal,
        hot_cold_rows_equal,
        hot_cold_roots_equal,
        status_sha256: digest(&hot_status)?,
        rows_sha256: digest(&hot_rows)?,
        event_root: hot_receipt.event_root,
        graph_digest: hot_receipt.graph_digest,
        event_set_root_v1: required_root(hot_receipt.event_set_root_v1, "event-set", n)?,
        projection_map_root_v2: required_root(
            hot_receipt.projection_map_root_v2,
            "projection-map",
            n,
        )?,
        enterprise_snapshot_root_v2: required_root(
            hot_receipt.enterprise_snapshot_root_v2,
            "enterprise-snapshot",
            n,
        )?,
    })
}

fn roots_equal(hot: &IndexReceipt, cold: &IndexReceipt) -> bool {
    hot.event_root == cold.event_root
        && hot.graph_digest == cold.graph_digest
        && hot.event_set_root_v1 == cold.event_set_root_v1
        && hot.projection_map_root_v2 == cold.projection_map_root_v2
        && hot.enterprise_snapshot_root_v2 == cold.enterprise_snapshot_root_v2
}

fn semantic_sample(sample: &ScaleSample) -> Value {
    json!({
        "observations_per_locator_before_append":
            sample.observations_per_locator_before_append,
        "seed_events": sample.seed_events,
        "hot_path_prime_counters": sample.hot_path_prime_counters,
        "appends": sample.appends,
        "hot_cold_status_equal": sample.hot_cold_status_equal,
        "hot_cold_rows_equal": sample.hot_cold_rows_equal,
        "hot_cold_roots_equal": sample.hot_cold_roots_equal,
        "status_sha256": sample.status_sha256,
        "rows_sha256": sample.rows_sha256,
        "event_root": sample.event_root,
        "graph_digest": sample.graph_digest,
        "event_set_root_v1": sample.event_set_root_v1,
        "projection_map_root_v2": sample.projection_map_root_v2,
        "enterprise_snapshot_root_v2": sample.enterprise_snapshot_root_v2,
    })
}

fn signed_delta(before: usize, after: usize) -> Result<i64, String> {
    let before = i64::try_from(before).map_err(|_| "row byte count exceeds i64".to_string())?;
    let after = i64::try_from(after).map_err(|_| "row byte count exceeds i64".to_string())?;
    after
        .checked_sub(before)
        .ok_or_else(|| "row byte delta overflow".to_string())
}

fn required_root(root: Option<String>, kind: &str, n: usize) -> Result<String, String> {
    root.ok_or_else(|| format!("high-fan-in hot receipt omitted {kind} root at N={n}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_uses_only_requested_scale() {
        assert_eq!(selected_sizes(64, false), vec![64]);
    }

    #[test]
    fn explicit_sweep_uses_enterprise_scales() {
        assert_eq!(selected_sizes(64, true), vec![1_000, 10_000, 100_000]);
    }
}
