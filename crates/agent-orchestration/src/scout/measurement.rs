use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_BOOTSTRAP_RESAMPLES: u32 = 10_000;
const MAX_BOOTSTRAP_RESAMPLES: u32 = 50_000;
const MAX_BOOTSTRAP_DRAWS: usize = 5_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementMethod {
    WilsonProportion,
    BootstrapMean,
    BootstrapMedian,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementComputation {
    pub sample_size: u64,
    pub missing: u64,
    pub estimate: f64,
    pub lower: f64,
    pub upper: f64,
    pub method: &'static str,
    pub method_version: &'static str,
    pub seed: Option<u64>,
    pub resamples: Option<u32>,
}

pub fn compute_measurement(
    method: MeasurementMethod,
    observations: &[Value],
    confidence: f64,
    resamples: Option<u32>,
    seed: Option<u64>,
) -> Result<MeasurementComputation, String> {
    if observations.is_empty() {
        return Err("measurement arrays must not be empty".into());
    }
    match method {
        MeasurementMethod::WilsonProportion => {
            if resamples.is_some() || seed.is_some() {
                return Err("Wilson measurements do not accept resamples or seed".into());
            }
            wilson_observations(observations, confidence)
        }
        MeasurementMethod::BootstrapMean | MeasurementMethod::BootstrapMedian => {
            bootstrap_observations(method, observations, confidence, resamples, seed)
        }
    }
}

fn wilson_observations(
    observations: &[Value],
    confidence: f64,
) -> Result<MeasurementComputation, String> {
    let mut successes = 0u64;
    let mut missing = 0u64;
    for observation in observations {
        match observation {
            Value::Null => missing += 1,
            Value::Bool(true) => successes += 1,
            Value::Bool(false) => {}
            Value::Number(number) if number.as_f64() == Some(1.0) => successes += 1,
            Value::Number(number) if number.as_f64() == Some(0.0) => {}
            _ => {
                return Err("Wilson arrays may contain only booleans, numeric 0/1, and null".into())
            }
        }
    }
    let sample_size =
        u64::try_from(observations.len()).map_err(|_| "measurement array is too large")?;
    let observed = sample_size - missing;
    if observed == 0 {
        return Err("Wilson measurements require at least one non-missing observation".into());
    }
    let z = confidence_z(confidence)
        .ok_or_else(|| "confidence must be 0.9, 0.95, or 0.99".to_string())?;
    let (estimate, lower, upper) = wilson(successes, observed, z);
    Ok(MeasurementComputation {
        sample_size,
        missing,
        estimate,
        lower,
        upper,
        method: "wilson_score",
        method_version: "scout-wilson-v1",
        seed: None,
        resamples: None,
    })
}

fn bootstrap_observations(
    method: MeasurementMethod,
    observations: &[Value],
    confidence: f64,
    resamples: Option<u32>,
    seed: Option<u64>,
) -> Result<MeasurementComputation, String> {
    let mut values = Vec::with_capacity(observations.len());
    let mut missing = 0u64;
    for observation in observations {
        match observation {
            Value::Null => missing += 1,
            Value::Number(number) => {
                let value = number
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| "bootstrap observations must be finite numbers".to_string())?;
                values.push(value);
            }
            _ => return Err("bootstrap arrays may contain only finite numbers and null".into()),
        }
    }
    if values.len() < 2 {
        return Err("bootstrap measurements require at least two observations".into());
    }
    let resamples = resamples.unwrap_or(DEFAULT_BOOTSTRAP_RESAMPLES);
    if !(100..=MAX_BOOTSTRAP_RESAMPLES).contains(&resamples) {
        return Err(format!(
            "bootstrap resamples must be between 100 and {MAX_BOOTSTRAP_RESAMPLES}"
        ));
    }
    let draws = values
        .len()
        .checked_mul(resamples as usize)
        .ok_or_else(|| "bootstrap work limit overflowed".to_string())?;
    if draws > MAX_BOOTSTRAP_DRAWS {
        return Err(format!(
            "bootstrap request exceeds the {MAX_BOOTSTRAP_DRAWS}-draw work limit"
        ));
    }
    let seed = seed.ok_or_else(|| "bootstrap measurements require an explicit seed".to_string())?;
    let estimate = statistic(method, &mut values.clone());
    let (lower, upper) = bootstrap_interval(method, &values, confidence, resamples, seed)?;
    Ok(MeasurementComputation {
        sample_size: u64::try_from(observations.len())
            .map_err(|_| "measurement array is too large")?,
        missing,
        estimate,
        lower,
        upper,
        method: match method {
            MeasurementMethod::BootstrapMean => "bootstrap_percentile_mean",
            MeasurementMethod::BootstrapMedian => "bootstrap_percentile_median",
            MeasurementMethod::WilsonProportion => {
                unreachable!("caller routed Wilson separately")
            }
        },
        method_version: "scout-bootstrap-v1",
        seed: Some(seed),
        resamples: Some(resamples),
    })
}

fn bootstrap_interval(
    method: MeasurementMethod,
    values: &[f64],
    confidence: f64,
    resamples: u32,
    seed: u64,
) -> Result<(f64, f64), String> {
    let alpha = (1.0 - confidence) / 2.0;
    if confidence_z(confidence).is_none() {
        return Err("confidence must be 0.9, 0.95, or 0.99".into());
    }
    let mut random = SplitMix64(seed);
    let mut sample = vec![0.0; values.len()];
    let mut statistics = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        for slot in &mut sample {
            *slot = values[random.index(values.len())];
        }
        statistics.push(statistic(method, &mut sample));
    }
    statistics.sort_by(f64::total_cmp);
    Ok((
        quantile(&statistics, alpha),
        quantile(&statistics, 1.0 - alpha),
    ))
}

fn statistic(method: MeasurementMethod, values: &mut [f64]) -> f64 {
    match method {
        MeasurementMethod::BootstrapMean => values.iter().sum::<f64>() / values.len() as f64,
        MeasurementMethod::BootstrapMedian => {
            values.sort_by(f64::total_cmp);
            let midpoint = values.len() / 2;
            if values.len() % 2 == 0 {
                (values[midpoint - 1] + values[midpoint]) / 2.0
            } else {
                values[midpoint]
            }
        }
        MeasurementMethod::WilsonProportion => {
            unreachable!("Wilson has a dedicated statistic")
        }
    }
}

fn quantile(sorted: &[f64], probability: f64) -> f64 {
    let rank = probability * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let weight = rank - lower as f64;
        sorted[lower] * (1.0 - weight) + sorted[upper] * weight
    }
}

fn confidence_z(confidence: f64) -> Option<f64> {
    if (confidence - 0.9).abs() < f64::EPSILON {
        Some(1.644_853_626_951_472_2)
    } else if (confidence - 0.95).abs() < f64::EPSILON {
        Some(1.959_963_984_540_054)
    } else if (confidence - 0.99).abs() < f64::EPSILON {
        Some(2.575_829_303_548_900_4)
    } else {
        None
    }
}

fn wilson(successes: u64, trials: u64, z: f64) -> (f64, f64, f64) {
    let n = trials as f64;
    let estimate = successes as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (estimate + z2 / (2.0 * n)) / denominator;
    let margin = z * ((estimate * (1.0 - estimate) / n + z2 / (4.0 * n * n)).sqrt()) / denominator;
    (
        estimate,
        (center - margin).max(0.0),
        (center + margin).min(1.0),
    )
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, length: usize) -> usize {
        ((u128::from(self.next()) * length as u128) >> 64) as usize
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn wilson_interval_matches_reference_fixture() {
        let observations = (0..100).map(|index| json!(index < 60)).collect::<Vec<_>>();
        let result = compute_measurement(
            MeasurementMethod::WilsonProportion,
            &observations,
            0.95,
            None,
            None,
        )
        .unwrap();
        assert!((result.estimate - 0.6).abs() < 1e-12);
        assert!((result.lower - 0.502_002_586_791_061_8).abs() < 1e-12);
        assert!((result.upper - 0.690_598_713_567_541_9).abs() < 1e-12);
    }

    #[test]
    fn seeded_bootstrap_is_deterministic_and_reports_missingness() {
        let observations = vec![json!(1.0), json!(2.0), Value::Null, json!(5.0)];
        let first = compute_measurement(
            MeasurementMethod::BootstrapMedian,
            &observations,
            0.95,
            Some(1_000),
            Some(42),
        )
        .unwrap();
        let second = compute_measurement(
            MeasurementMethod::BootstrapMedian,
            &observations,
            0.95,
            Some(1_000),
            Some(42),
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.sample_size, 4);
        assert_eq!(first.missing, 1);
        assert_eq!(first.estimate, 2.0);
        assert!(first.lower <= first.estimate && first.estimate <= first.upper);
    }

    #[test]
    fn confidence_is_an_explicit_closed_set() {
        let observations = vec![json!(1.0), json!(2.0)];
        assert!(compute_measurement(
            MeasurementMethod::BootstrapMean,
            &observations,
            0.951,
            Some(100),
            Some(1)
        )
        .unwrap_err()
        .contains("confidence"));
    }
}
