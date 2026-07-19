use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;

use super::model::{EvidenceLevel, RunRecord};

#[derive(Clone, Debug, Serialize)]
pub struct LaneAggregate {
    pub lane_id: String,
    pub runs: usize,
    pub pass_rate: f64,
    pub behavioral_correctness: f64,
    pub lifecycle_conformance: f64,
    pub efficiency_score: f64,
    pub avg_tokens: f64,
    pub avg_cost_usd: f64,
    pub avg_wall_ms: f64,
    pub avg_agent_ms: f64,
    pub avg_model_polling_tokens: f64,
    pub avg_duplicate_setup_tokens: f64,
    pub verified_successes_per_100k_tokens: f64,
    pub hard_failures: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValueComparison {
    pub lane_id: String,
    pub control_lane_id: String,
    pub pass_rate_delta: f64,
    pub lifecycle_delta: f64,
    pub token_ratio: f64,
    pub cost_ratio: f64,
    pub wall_time_ratio: f64,
    pub verified_yield_delta: f64,
    pub value_gate_passed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BenchmarkSummary {
    pub schema_version: u32,
    pub evidence_level: EvidenceLevel,
    pub total_runs: usize,
    pub required_graph_pass_rate: f64,
    pub aggregates: Vec<LaneAggregate>,
    pub comparisons: Vec<ValueComparison>,
    pub value_claim_allowed: bool,
    pub note: String,
}

pub fn write_report(
    output_root: &Path,
    records: &[RunRecord],
) -> Result<BenchmarkSummary, Box<dyn std::error::Error + Send + Sync>> {
    let evidence_level = records
        .first()
        .map(|record| record.evidence_level)
        .unwrap_or(EvidenceLevel::Simulation);
    let mut lanes = BTreeMap::<String, Vec<&RunRecord>>::new();
    for record in records {
        lanes
            .entry(record.lane.id.clone())
            .or_default()
            .push(record);
    }
    let aggregates = lanes
        .iter()
        .map(|(lane, lane_records)| aggregate(lane, lane_records))
        .collect::<Vec<_>>();
    let control = aggregates
        .iter()
        .find(|lane| lane.lane_id == "equal-budget-single");
    let comparisons: Vec<ValueComparison> = control
        .map(|control| {
            aggregates
                .iter()
                .filter(|lane| lane.lane_id.starts_with("work-graph-"))
                .map(|lane| compare(lane, control))
                .collect()
        })
        .unwrap_or_default();
    let required = records
        .iter()
        .filter(|record| record.lane.is_work_graph())
        .collect::<Vec<_>>();
    let required_graph_pass_rate = fraction(
        required.iter().filter(|record| record.passed()).count(),
        required.len(),
    );
    let minimum_repetitions = records
        .iter()
        .map(|record| record.repetition)
        .max()
        .map_or(0, |maximum| maximum + 1);
    let production_receipts = records
        .iter()
        .filter(|record| record.lane.is_work_graph())
        .all(|record| record.result.production_trace_id.is_some());
    let value_claim_allowed = evidence_level == EvidenceLevel::ExternalTrace
        && minimum_repetitions >= 3
        && production_receipts
        && comparisons
            .iter()
            .any(|comparison| comparison.value_gate_passed);
    let note = if evidence_level == EvidenceLevel::Simulation {
        "Simulation proves fixture solvability and rubric behavior only. It cannot establish model quality, autonomy, or that more tokens improve production outcomes."
    } else if minimum_repetitions < 3 {
        "External traces require at least three paired repetitions before a value claim is allowed."
    } else if !production_receipts {
        "External output is missing production-host trace identities; self-reported lifecycle claims are not sufficient."
    } else {
        "Value is gated against the equal-token single-agent control, not against an artificially smaller budget."
    }
    .to_string();
    let summary = BenchmarkSummary {
        schema_version: 1,
        evidence_level,
        total_runs: records.len(),
        required_graph_pass_rate,
        aggregates,
        comparisons,
        value_claim_allowed,
        note,
    };
    fs::write(
        output_root.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    fs::write(output_root.join("report.md"), markdown(&summary, records))?;
    Ok(summary)
}

fn aggregate(lane_id: &str, records: &[&RunRecord]) -> LaneAggregate {
    let runs = records.len();
    let total_tokens = records
        .iter()
        .map(|record| record.total_tokens() as f64)
        .sum::<f64>();
    let mut hard_failures = BTreeMap::new();
    for record in records {
        for failure in &record.hard_failures {
            *hard_failures.entry(format!("{failure:?}")).or_insert(0) += 1;
        }
    }
    LaneAggregate {
        lane_id: lane_id.into(),
        runs,
        pass_rate: fraction(
            records.iter().filter(|record| record.passed()).count(),
            runs,
        ),
        behavioral_correctness: mean(records, |record| record.behavioral_correctness),
        lifecycle_conformance: mean(records, |record| record.lifecycle_conformance),
        efficiency_score: mean(records, |record| record.efficiency_score),
        avg_tokens: average_sum(records, |record| record.total_tokens() as f64),
        avg_cost_usd: average_sum(records, |record| record.result.usage.cost_usd),
        avg_wall_ms: average_sum(records, |record| record.result.usage.wall_ms as f64),
        avg_agent_ms: average_sum(records, |record| record.result.usage.agent_ms as f64),
        avg_model_polling_tokens: average_sum(records, |record| {
            record.result.usage.model_polling_tokens as f64
        }),
        avg_duplicate_setup_tokens: average_sum(records, |record| {
            record.result.usage.duplicate_setup_tokens as f64
        }),
        verified_successes_per_100k_tokens: if total_tokens == 0.0 {
            0.0
        } else {
            records.iter().filter(|record| record.passed()).count() as f64 * 100_000.0
                / total_tokens
        },
        hard_failures,
    }
}

fn compare(candidate: &LaneAggregate, control: &LaneAggregate) -> ValueComparison {
    let token_ratio = ratio(candidate.avg_tokens, control.avg_tokens);
    let cost_ratio = ratio(candidate.avg_cost_usd, control.avg_cost_usd);
    let wall_time_ratio = ratio(candidate.avg_wall_ms, control.avg_wall_ms);
    let pass_rate_delta = candidate.pass_rate - control.pass_rate;
    let lifecycle_delta = candidate.lifecycle_conformance - control.lifecycle_conformance;
    let verified_yield_delta =
        candidate.verified_successes_per_100k_tokens - control.verified_successes_per_100k_tokens;
    ValueComparison {
        lane_id: candidate.lane_id.clone(),
        control_lane_id: control.lane_id.clone(),
        pass_rate_delta,
        lifecycle_delta,
        token_ratio,
        cost_ratio,
        wall_time_ratio,
        verified_yield_delta,
        value_gate_passed: pass_rate_delta >= 0.10
            && lifecycle_delta > 0.0
            && verified_yield_delta > 0.0
            && token_ratio <= 1.25
            && cost_ratio <= 1.25
            && wall_time_ratio <= 0.90
            && candidate.avg_model_polling_tokens == 0.0,
    }
}

fn markdown(summary: &BenchmarkSummary, records: &[RunRecord]) -> String {
    let mut out = String::from("# Clark universal work-graph orchestration benchmark\n\n");
    out.push_str(&format!(
        "Evidence: `{:?}`  \nRuns: {}  \nRequired work-graph pass rate: {:.1}%  \nValue claim allowed: **{}**\n\n{}\n\n",
        summary.evidence_level,
        summary.total_runs,
        summary.required_graph_pass_rate * 100.0,
        summary.value_claim_allowed,
        summary.note
    ));
    out.push_str("## Lane results\n\n| Lane | Pass | Behavior | Lifecycle | Efficiency | Tokens | Polling | Duplicate setup | Wall ms | Agent ms | Verified / 100k |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for lane in &summary.aggregates {
        out.push_str(&format!(
            "| {} | {:.0}% | {:.0}% | {:.0}% | {:.0}% | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} |\n",
            lane.lane_id,
            lane.pass_rate * 100.0,
            lane.behavioral_correctness * 100.0,
            lane.lifecycle_conformance * 100.0,
            lane.efficiency_score * 100.0,
            lane.avg_tokens,
            lane.avg_model_polling_tokens,
            lane.avg_duplicate_setup_tokens,
            lane.avg_wall_ms,
            lane.avg_agent_ms,
            lane.verified_successes_per_100k_tokens,
        ));
    }
    out.push_str("\n## Budget-matched single-agent comparisons\n\n| Lane | Pass delta | Lifecycle delta | Token ratio | Cost ratio | Wall ratio | Yield delta | Gate |\n|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for comparison in &summary.comparisons {
        out.push_str(&format!(
            "| {} | {:+.0}% | {:+.0}% | {:.2}x | {:.2}x | {:.2}x | {:+.2} | {} |\n",
            comparison.lane_id,
            comparison.pass_rate_delta * 100.0,
            comparison.lifecycle_delta * 100.0,
            comparison.token_ratio,
            comparison.cost_ratio,
            comparison.wall_time_ratio,
            comparison.verified_yield_delta,
            comparison.value_gate_passed,
        ));
    }
    out.push_str("\n## Failing runs\n\n");
    let mut failures = 0;
    for record in records.iter().filter(|record| !record.passed()) {
        failures += 1;
        out.push_str(&format!(
            "- `{}` / `{}`: {:?}\n",
            record.scenario_id, record.lane.id, record.hard_failures
        ));
    }
    if failures == 0 {
        out.push_str("No failing runs.\n");
    }
    out
}

fn average_sum(records: &[&RunRecord], value: impl Fn(&RunRecord) -> f64) -> f64 {
    if records.is_empty() {
        0.0
    } else {
        records.iter().map(|record| value(record)).sum::<f64>() / records.len() as f64
    }
}

fn mean(records: &[&RunRecord], value: impl Fn(&RunRecord) -> f64) -> f64 {
    average_sum(records, value)
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}
