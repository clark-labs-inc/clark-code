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
    pub replay_correctness: f64,
    pub conformance_score: f64,
    pub avg_tokens: f64,
    pub avg_cost_usd: f64,
    pub avg_wall_ms: f64,
    pub useful_token_ratio: f64,
    pub duplicate_read_ratio: f64,
    pub verified_successes_per_100k_tokens: f64,
    pub hard_failures: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValueComparison {
    pub lane_id: String,
    pub control_lane_id: String,
    pub pass_rate_delta: f64,
    pub behavioral_delta: f64,
    pub token_ratio: f64,
    pub cost_delta_usd: f64,
    pub wall_time_ratio: f64,
    pub verified_yield_delta: f64,
    pub value_gate_passed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BenchmarkSummary {
    pub schema_version: u32,
    pub evidence_level: EvidenceLevel,
    pub total_runs: usize,
    pub multi_conformance_pass_rate: f64,
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
        .unwrap_or(EvidenceLevel::Scripted);
    let mut by_lane = BTreeMap::<String, Vec<&RunRecord>>::new();
    for record in records {
        by_lane
            .entry(record.lane.id.clone())
            .or_default()
            .push(record);
    }
    let aggregates = by_lane
        .iter()
        .map(|(id, lane_records)| aggregate(id, lane_records))
        .collect::<Vec<_>>();
    let control = aggregates
        .iter()
        .find(|aggregate| aggregate.lane_id == "equal-budget-single");
    let comparisons = aggregates
        .iter()
        .filter(|aggregate| {
            aggregate.lane_id.starts_with("multi-") || aggregate.lane_id == "cloud-mixed"
        })
        .filter_map(|aggregate| control.map(|control| compare(aggregate, control)))
        .collect::<Vec<_>>();
    let multi_records = records
        .iter()
        .filter(|record| record.lane.is_multi())
        .collect::<Vec<_>>();
    let multi_conformance_pass_rate = fraction(
        multi_records
            .iter()
            .filter(|record| record.conformance_score >= 1.0 && record.replay_correctness >= 1.0)
            .count(),
        multi_records.len(),
    );
    let value_claim_allowed = evidence_level == EvidenceLevel::External
        && comparisons
            .iter()
            .any(|comparison| comparison.value_gate_passed)
        && records
            .iter()
            .map(|record| record.repetition)
            .max()
            .unwrap_or(0)
            >= 2;
    let note = if evidence_level == EvidenceLevel::External {
        "External candidate evidence. A value claim still requires at least three repetitions and a positive correctness-adjusted yield gate."
    } else {
        "Scripted conformance evidence only. It proves benchmark mechanics and lifecycle gaps, not that more model tokens improve outcomes."
    }.to_string();
    let summary = BenchmarkSummary {
        schema_version: 1,
        evidence_level,
        total_runs: records.len(),
        multi_conformance_pass_rate,
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
    let tokens = records
        .iter()
        .map(|record| record.total_tokens() as f64)
        .sum::<f64>();
    let useful = records
        .iter()
        .map(|record| record.result.usage.useful_tokens as f64)
        .sum::<f64>();
    let duplicate = records
        .iter()
        .map(|record| record.result.usage.duplicate_read_tokens as f64)
        .sum::<f64>();
    let mut hard_failures = BTreeMap::new();
    for record in records {
        for failure in &record.hard_failures {
            *hard_failures.entry(format!("{failure:?}")).or_insert(0) += 1;
        }
    }
    let pass_rate = fraction(
        records.iter().filter(|record| record.passed()).count(),
        runs,
    );
    LaneAggregate {
        lane_id: lane_id.into(),
        runs,
        pass_rate,
        behavioral_correctness: mean(records, |record| record.behavioral_correctness),
        replay_correctness: mean(records, |record| record.replay_correctness),
        conformance_score: mean(records, |record| record.conformance_score),
        avg_tokens: if runs == 0 { 0.0 } else { tokens / runs as f64 },
        avg_cost_usd: mean(records, |record| record.result.usage.cost_usd),
        avg_wall_ms: mean(records, |record| record.result.usage.wall_ms as f64),
        useful_token_ratio: if tokens == 0.0 { 0.0 } else { useful / tokens },
        duplicate_read_ratio: if tokens == 0.0 {
            0.0
        } else {
            duplicate / tokens
        },
        verified_successes_per_100k_tokens: if tokens == 0.0 {
            0.0
        } else {
            records.iter().filter(|record| record.passed()).count() as f64 * 100_000.0 / tokens
        },
        hard_failures,
    }
}

fn compare(candidate: &LaneAggregate, control: &LaneAggregate) -> ValueComparison {
    let token_ratio = ratio(candidate.avg_tokens, control.avg_tokens);
    let wall_time_ratio = ratio(candidate.avg_wall_ms, control.avg_wall_ms);
    let pass_rate_delta = candidate.pass_rate - control.pass_rate;
    let verified_yield_delta =
        candidate.verified_successes_per_100k_tokens - control.verified_successes_per_100k_tokens;
    ValueComparison {
        lane_id: candidate.lane_id.clone(),
        control_lane_id: control.lane_id.clone(),
        pass_rate_delta,
        behavioral_delta: candidate.behavioral_correctness - control.behavioral_correctness,
        token_ratio,
        cost_delta_usd: candidate.avg_cost_usd - control.avg_cost_usd,
        wall_time_ratio,
        verified_yield_delta,
        value_gate_passed: pass_rate_delta >= 0.10
            && verified_yield_delta > 0.0
            && token_ratio <= 1.25,
    }
}

fn markdown(summary: &BenchmarkSummary, records: &[RunRecord]) -> String {
    let mut out = String::from("# Agent multi-repository orchestration benchmark\n\n");
    out.push_str(&format!("Evidence: `{:?}`  \nRuns: {}  \nMulti-agent conformance: {:.1}%  \nValue claim allowed: **{}**\n\n{}\n\n", summary.evidence_level, summary.total_runs, summary.multi_conformance_pass_rate * 100.0, summary.value_claim_allowed, summary.note));
    out.push_str("## Lane results\n\n| Lane | Pass | Behavior | Replay | Conformance | Tokens | Useful | Duplicate | Wall ms | Verified / 100k |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for lane in &summary.aggregates {
        out.push_str(&format!("| {} | {:.0}% | {:.0}% | {:.0}% | {:.0}% | {:.0} | {:.0}% | {:.0}% | {:.0} | {:.2} |\n", lane.lane_id, lane.pass_rate * 100.0, lane.behavioral_correctness * 100.0, lane.replay_correctness * 100.0, lane.conformance_score * 100.0, lane.avg_tokens, lane.useful_token_ratio * 100.0, lane.duplicate_read_ratio * 100.0, lane.avg_wall_ms, lane.verified_successes_per_100k_tokens));
    }
    out.push_str("\n## Equal-token comparisons\n\n| Lane | Pass delta | Behavior delta | Token ratio | Wall ratio | Yield delta | Gate |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for comparison in &summary.comparisons {
        out.push_str(&format!(
            "| {} | {:+.0}% | {:+.0}% | {:.2}x | {:.2}x | {:+.2} | {} |\n",
            comparison.lane_id,
            comparison.pass_rate_delta * 100.0,
            comparison.behavioral_delta * 100.0,
            comparison.token_ratio,
            comparison.wall_time_ratio,
            comparison.verified_yield_delta,
            comparison.value_gate_passed
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

fn mean(records: &[&RunRecord], value: impl Fn(&RunRecord) -> f64) -> f64 {
    if records.is_empty() {
        0.0
    } else {
        records.iter().map(|record| value(record)).sum::<f64>() / records.len() as f64
    }
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
