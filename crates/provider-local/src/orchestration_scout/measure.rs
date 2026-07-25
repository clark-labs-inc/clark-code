use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::domain::ToolKind;
use agent_orchestration::{
    compute_scout_measurement, ScoutConfidenceInterval, ScoutEvidenceArtifact, ScoutEvidenceId,
    ScoutEvidenceKind, ScoutMeasurement, ScoutMeasurementMethod, ScoutRunnerId,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::probe::{resolve_probe_path, sensitive_path};
use super::ScoutToolState;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

const MAX_MEASURE_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSON_POINTER_BYTES: usize = 1_024;

pub(super) struct ScoutMeasureTool {
    pub state: Arc<ScoutToolState>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureArgs {
    method: ScoutMeasurementMethod,
    run_id: String,
    evidence_id: String,
    scope: String,
    source_evidence_ids: BTreeSet<ScoutEvidenceId>,
    path: String,
    json_pointer: String,
    confidence: f64,
    #[serde(default)]
    resamples: Option<u32>,
    #[serde(default)]
    seed: Option<u64>,
}

#[derive(Serialize)]
struct MeasurementReceipt<'a> {
    schema_version: &'static str,
    source_evidence_ids: &'a BTreeSet<ScoutEvidenceId>,
    path: &'a str,
    json_pointer: &'a str,
    input_sha256: &'a str,
    sample_size: u64,
    missing: u64,
    estimate: f64,
    lower: f64,
    upper: f64,
    confidence: f64,
    method: &'static str,
    method_version: &'static str,
    seed: Option<u64>,
    resamples: Option<u32>,
}

#[async_trait]
impl ToolExecutor for ScoutMeasureTool {
    fn name(&self) -> &str {
        "scout_measure"
    }

    fn description(&self) -> &str {
        "Read a bounded project JSON array and compute a deterministic Wilson proportion or seeded bootstrap mean/median interval in Rust, then append host-owned measurement evidence. At least one verified source artifact must bind the same path and scope. Raw observations, estimates, intervals, power, and proof tiers are never accepted from the model."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["wilson_proportion", "bootstrap_mean", "bootstrap_median"],
                    "description": "Choose the statistical construct before identifying its data."
                },
                "run_id": {"type": "string"},
                "evidence_id": {"type": "string"},
                "scope": {"type": "string"},
                "source_evidence_ids": {
                    "type": "array",
                    "minItems": 1,
                    "uniqueItems": true,
                    "items": {"type": "string"}
                },
                "path": {
                    "type": "string",
                    "description": "Project-relative JSON file bound by verified source evidence."
                },
                "json_pointer": {
                    "type": "string",
                    "description": "RFC 6901 pointer to an array. Wilson accepts booleans, 0/1, and null; bootstrap accepts finite numbers and null."
                },
                "confidence": {"type": "number", "enum": [0.9, 0.95, 0.99]},
                "resamples": {
                    "type": "integer",
                    "minimum": 100,
                    "maximum": 50000,
                    "description": "Bootstrap resample count; omit for Wilson."
                },
                "seed": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Required for bootstrap reproducibility; omit for Wilson."
                }
            },
            "required": [
                "method",
                "run_id",
                "evidence_id",
                "scope",
                "source_evidence_ids",
                "path",
                "json_pointer",
                "confidence"
            ],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: MeasureArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(format!("invalid Scout measurement: {error}")),
        };
        if args.source_evidence_ids.is_empty() {
            return ToolOutcome::error("source_evidence_ids must not be empty");
        }
        if args.json_pointer.len() > MAX_JSON_POINTER_BYTES {
            return ToolOutcome::error("json_pointer exceeds the 1024-byte limit");
        }
        let evidence_id = match ScoutEvidenceId::new(&args.evidence_id) {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        let path = match resolve_probe_path(ctx, &args.path) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        if sensitive_path(&path) {
            return ToolOutcome::error(
                "scout_measure refuses secret-bearing paths; measure a redacted fixture instead",
            );
        }
        let display_path = ctx.sandbox.display(&path);
        let bytes = match ctx.executor.read(&path).await {
            Ok(bytes) => bytes,
            Err(error) => return ToolOutcome::error(error),
        };
        if bytes.len() > MAX_MEASURE_BYTES {
            return ToolOutcome::error(format!(
                "measurement input exceeds the {MAX_MEASURE_BYTES} byte limit"
            ));
        }
        let input_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let document: Value = match serde_json::from_slice(&bytes) {
            Ok(document) => document,
            Err(error) => {
                return ToolOutcome::error(format!("measurement input is not JSON: {error}"))
            }
        };
        let Some(observations) = document
            .pointer(&args.json_pointer)
            .and_then(Value::as_array)
        else {
            return ToolOutcome::error(
                "json_pointer must resolve to an array in the measurement input",
            );
        };
        let result = match compute_scout_measurement(
            args.method,
            observations,
            args.confidence,
            args.resamples,
            args.seed,
        ) {
            Ok(result) => result,
            Err(error) => return ToolOutcome::error(error),
        };

        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let Some(ledger) = ledgers.get_mut(&args.run_id) else {
            return ToolOutcome::error(format!("unknown Scout run {}", args.run_id));
        };
        if !ledger.snapshot().charter.scopes.contains(&args.scope) {
            return ToolOutcome::error(format!("undeclared Scout scope {}", args.scope));
        }
        let mut path_bound = false;
        for source_id in &args.source_evidence_ids {
            let Some(record) = ledger.snapshot().evidence.get(source_id) else {
                return ToolOutcome::error(format!("unknown source evidence {source_id}"));
            };
            if !verified(record) {
                return ToolOutcome::error(format!(
                    "source evidence {source_id} has not passed host verification"
                ));
            }
            if record.artifact.scope != args.scope {
                return ToolOutcome::error(format!(
                    "source evidence {source_id} belongs to a different scope"
                ));
            }
            path_bound |= source_binds_path(&record.artifact.source, &display_path, &args.path);
        }
        if !path_bound {
            return ToolOutcome::error(
                "at least one verified source artifact must bind the measurement path",
            );
        }
        let receipt = MeasurementReceipt {
            schema_version: "scout-measurement-receipt-v1",
            source_evidence_ids: &args.source_evidence_ids,
            path: &display_path,
            json_pointer: &args.json_pointer,
            input_sha256: &input_sha256,
            sample_size: result.sample_size,
            missing: result.missing,
            estimate: result.estimate,
            lower: result.lower,
            upper: result.upper,
            confidence: args.confidence,
            method: result.method,
            method_version: result.method_version,
            seed: result.seed,
            resamples: result.resamples,
        };
        let encoded = serde_json::to_vec(&receipt).unwrap_or_default();
        let digest = format!("{:x}", Sha256::digest(encoded));
        let recipe = json!({
            "schema_version": "scout-measurement-recipe-v1",
            "method": args.method,
            "path": display_path,
            "json_pointer": args.json_pointer,
            "confidence": args.confidence,
            "seed": result.seed,
            "resamples": result.resamples,
        });
        let artifact = ScoutEvidenceArtifact {
            id: evidence_id,
            kind: ScoutEvidenceKind::Measurement,
            source: format!("scout_measure:{}#{}", display_path, args.json_pointer),
            content_sha256: digest.clone(),
            observed_at_ms: now_ms(),
            snapshot_id: ledger.snapshot().charter.snapshot_id.clone(),
            scope: args.scope,
            recipe: Some(recipe.to_string()),
            proof_tier: Some(agent_orchestration::ProofTier::T1Source),
            measurement: Some(ScoutMeasurement {
                sample_size: result.sample_size,
                missing: result.missing,
                estimate: result.estimate,
                interval: ScoutConfidenceInterval {
                    lower: result.lower,
                    upper: result.upper,
                    confidence: args.confidence,
                },
                method: result.method.into(),
                method_version: result.method_version.into(),
                seed: result.seed,
                power: None,
            }),
            offline_poc_controls: None,
            reproduces: None,
        };
        let runner_id = ScoutRunnerId::new(format!("measure:{}", Uuid::new_v4()))
            .expect("valid generated runner id");
        if let Err(error) = ledger.record_evidence(artifact, runner_id) {
            return ToolOutcome::error(error);
        }
        ToolOutcome::ok(format!(
            "Measurement `{}` recorded from {} observations ({} missing): {:.6}, {:.1}% CI [{:.6}, {:.6}] via {}.",
            args.evidence_id,
            result.sample_size,
            result.missing,
            result.estimate,
            args.confidence * 100.0,
            result.lower,
            result.upper,
            result.method,
        ))
        .with_details(json!({
            "content_sha256": digest,
            "input_sha256": input_sha256,
            "sample_size": result.sample_size,
            "missing": result.missing,
            "estimate": result.estimate,
            "interval": {
                "lower": result.lower,
                "upper": result.upper,
                "confidence": args.confidence
            },
            "method": result.method,
            "method_version": result.method_version,
            "seed": result.seed,
            "resamples": result.resamples
        }))
    }
}

fn verified(record: &agent_orchestration::ScoutEvidenceRecord) -> bool {
    match record.checks.last() {
        Some(check) => matches!(
            check.outcome,
            agent_orchestration::VerificationOutcome::Exact
                | agent_orchestration::VerificationOutcome::Equivalent
        ),
        None => matches!(
            record.producer,
            agent_orchestration::EvidenceProducer::Runner { .. }
        ),
    }
}

fn source_binds_path(source: &str, display_path: &str, requested_path: &str) -> bool {
    [display_path, requested_path]
        .into_iter()
        .map(|path| path.trim_start_matches("./"))
        .any(|path| {
            let source = source.trim_start_matches("./");
            source == path
                || source.starts_with(&format!("{path}:"))
                || source.starts_with(&format!("{path}#"))
        })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wilson_interval_matches_reference_fixture() {
        let observations = (0..100).map(|index| json!(index < 60)).collect::<Vec<_>>();
        let result = compute_scout_measurement(
            ScoutMeasurementMethod::WilsonProportion,
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
    fn measurement_sources_are_bound_to_the_same_path() {
        assert!(source_binds_path(
            "data/metrics.json:json_array_count",
            "data/metrics.json",
            "./data/metrics.json"
        ));
        assert!(!source_binds_path(
            "data/other.json:json_array_count",
            "data/metrics.json",
            "data/metrics.json"
        ));
    }
}
