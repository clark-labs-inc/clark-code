use crate::model::{CaseRecord, LaneSummary, PairedEffect};
use std::collections::BTreeMap;

pub fn lane_summaries(records: &[CaseRecord]) -> Vec<LaneSummary> {
    let mut grouped: BTreeMap<&str, Vec<&CaseRecord>> = BTreeMap::new();
    for record in records {
        grouped.entry(&record.lane).or_default().push(record);
    }
    grouped
        .into_iter()
        .map(|(lane, rows)| {
            let cases = rows.len();
            LaneSummary {
                lane: lane.to_string(),
                cases,
                mean_hidden_check_score: mean(&rows, |row| row.verification.score()),
                retrieval_compliance_rate: mean_optional(&rows, |row| {
                    row.retrieval_treatment.applicable.then_some(
                        if row.retrieval_treatment.compliant {
                            1.0
                        } else {
                            0.0
                        },
                    )
                }),
                hidden_check_full_success_rate: mean(&rows, |row| {
                    if row.verification.score() == 1.0 {
                        1.0
                    } else {
                        0.0
                    }
                }),
                mean_total_tokens: mean(&rows, |row| {
                    (row.planner_usage.input_tokens
                        + row.planner_usage.output_tokens
                        + row.executor_usage.input_tokens
                        + row.executor_usage.output_tokens) as f64
                }),
                mean_latency_ms: mean(&rows, |row| {
                    (row.planner_usage.elapsed_ms + row.executor_usage.elapsed_ms) as f64
                }),
                total_cost_usd: rows
                    .iter()
                    .map(|row| row.planner_usage.cost_usd + row.executor_usage.cost_usd)
                    .sum(),
            }
        })
        .collect()
}

pub fn paired_effect(
    records: &[CaseRecord],
    control: &str,
    candidate: &str,
) -> Option<PairedEffect> {
    let keyed = |lane: &str| {
        records
            .iter()
            .filter(|row| row.lane == lane)
            .map(|row| {
                (
                    (row.scenario.as_str(), row.repetition),
                    row.verification.score(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let controls = keyed(control);
    let candidates = keyed(candidate);
    let deltas = controls
        .iter()
        .filter_map(|(key, control_score)| {
            candidates
                .get(key)
                .map(|candidate_score| (key.0.to_string(), candidate_score - control_score))
        })
        .collect::<Vec<_>>();
    if deltas.is_empty() {
        return None;
    }
    let (low, high) = hierarchical_bootstrap_ci(&deltas, 10_000);
    Some(PairedEffect {
        control: control.to_string(),
        candidate: candidate.to_string(),
        pairs: deltas.len(),
        mean_executor_delta: scenario_balanced_mean(&deltas),
        ci95_low: low,
        ci95_high: high,
    })
}

fn mean(rows: &[&CaseRecord], metric: impl Fn(&CaseRecord) -> f64) -> f64 {
    rows.iter().map(|row| metric(row)).sum::<f64>() / rows.len() as f64
}

fn mean_optional(rows: &[&CaseRecord], metric: impl Fn(&CaseRecord) -> Option<f64>) -> Option<f64> {
    let values = rows
        .iter()
        .filter_map(|row| metric(row))
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn scenario_balanced_mean(values: &[(String, f64)]) -> f64 {
    let mut grouped: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (scenario, value) in values {
        grouped.entry(scenario).or_default().push(*value);
    }
    grouped
        .values()
        .map(|items| items.iter().sum::<f64>() / items.len() as f64)
        .sum::<f64>()
        / grouped.len() as f64
}

fn hierarchical_bootstrap_ci(values: &[(String, f64)], iterations: usize) -> (f64, f64) {
    let mut grouped: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (scenario, value) in values {
        grouped.entry(scenario).or_default().push(*value);
    }
    if grouped.len() == 1
        && grouped
            .values()
            .next()
            .is_some_and(|items| items.len() == 1)
    {
        let value = grouped.values().next().unwrap()[0];
        return (value, value);
    }
    let scenarios = grouped.values().collect::<Vec<_>>();
    let mut state = 0x5eed_1234_89ab_cdef_u64;
    let mut means = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let scenario_sum = (0..scenarios.len())
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let repetitions = scenarios[(state as usize) % scenarios.len()];
                let repetition_sum = (0..repetitions.len())
                    .map(|_| {
                        state = state
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1);
                        repetitions[(state as usize) % repetitions.len()]
                    })
                    .sum::<f64>();
                repetition_sum / repetitions.len() as f64
            })
            .sum::<f64>();
        means.push(scenario_sum / scenarios.len() as f64);
    }
    means.sort_by(f64::total_cmp);
    (
        means[((iterations as f64 * 0.025) as usize).min(iterations - 1)],
        means[((iterations as f64 * 0.975) as usize).min(iterations - 1)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_is_deterministic_and_bounded() {
        let values = [
            ("a".into(), -0.5),
            ("a".into(), 0.0),
            ("b".into(), 0.5),
            ("b".into(), 1.0),
        ];
        let first = hierarchical_bootstrap_ci(&values, 1_000);
        let second = hierarchical_bootstrap_ci(&values, 1_000);
        assert_eq!(first, second);
        assert!(first.0 <= 0.25 && first.1 >= 0.25);
    }
}
