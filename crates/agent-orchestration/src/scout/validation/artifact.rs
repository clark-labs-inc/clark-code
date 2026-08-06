use super::super::{
    EvidenceArtifact, EvidenceKind, Measurement, OfflinePocControls, ProofTier, ScoutSnapshot,
};
use super::{nonempty, validate_digest};

pub(super) fn validate_artifact(
    snapshot: &ScoutSnapshot,
    artifact: &EvidenceArtifact,
) -> Result<(), String> {
    nonempty("evidence source", &artifact.source)?;
    nonempty("evidence snapshot_id", &artifact.snapshot_id)?;
    nonempty("evidence scope", &artifact.scope)?;
    if artifact.snapshot_id != snapshot.charter.snapshot_id {
        return Err(format!(
            "evidence {} was collected against a different snapshot",
            artifact.id
        ));
    }
    if !snapshot.charter.scopes.contains(&artifact.scope) {
        return Err(format!(
            "evidence {} uses undeclared scope {}",
            artifact.id, artifact.scope
        ));
    }
    if artifact.observed_at_ms == 0 {
        return Err(format!(
            "evidence {} requires an observation timestamp",
            artifact.id
        ));
    }
    validate_digest(
        &format!("evidence {}", artifact.id),
        &artifact.content_sha256,
    )?;
    match &artifact.recipe {
        Some(recipe) => nonempty("evidence recipe", recipe)?,
        None if artifact.kind != EvidenceKind::Assumption => {
            return Err(format!("evidence {} requires a replay recipe", artifact.id))
        }
        None => {}
    }
    if let (Some(tier), Some(maximum)) = (artifact.proof_tier, artifact.kind.maximum_tier()) {
        if tier > maximum {
            return Err(format!(
                "evidence {} claims {:?} above the {:?} ceiling for {:?}",
                artifact.id, tier, maximum, artifact.kind
            ));
        }
    }
    match (&artifact.kind, &artifact.measurement) {
        (EvidenceKind::Measurement, Some(measurement)) => validate_measurement(measurement)?,
        (EvidenceKind::Measurement, None) => {
            return Err(format!(
                "measurement evidence {} requires typed measurement data",
                artifact.id
            ))
        }
        (_, Some(_)) => {
            return Err(format!(
                "only measurement evidence may carry typed measurement data: {}",
                artifact.id
            ))
        }
        _ => {}
    }
    match (
        &artifact.kind,
        artifact.proof_tier,
        &artifact.offline_poc_controls,
    ) {
        (
            EvidenceKind::OfflinePoc | EvidenceKind::Counterexample,
            Some(ProofTier::T3OfflinePoc),
            Some(controls),
        ) => validate_offline_poc_controls(controls)?,
        (
            EvidenceKind::OfflinePoc | EvidenceKind::Counterexample,
            Some(ProofTier::T3OfflinePoc),
            None,
        ) => {
            return Err(format!(
                "T3 evidence {} requires typed positive and negative controls",
                artifact.id
            ))
        }
        (EvidenceKind::OfflinePoc | EvidenceKind::Counterexample, _, Some(controls)) => {
            validate_offline_poc_controls(controls)?
        }
        (_, _, Some(_)) => {
            return Err(format!(
                "only offline PoC evidence may carry control receipts: {}",
                artifact.id
            ))
        }
        _ => {}
    }
    if artifact.kind == EvidenceKind::Reproduction && artifact.reproduces.is_none() {
        return Err(format!(
            "reproduction evidence {} must identify what it reproduces",
            artifact.id
        ));
    }
    if artifact.kind != EvidenceKind::Reproduction && artifact.reproduces.is_some() {
        return Err(format!(
            "only reproduction evidence may set reproduces: {}",
            artifact.id
        ));
    }
    Ok(())
}

fn validate_measurement(measurement: &Measurement) -> Result<(), String> {
    if measurement.sample_size == 0 {
        return Err("measurements require sample_size > 0".to_string());
    }
    if measurement.missing > measurement.sample_size {
        return Err("measurement missing count cannot exceed sample_size".to_string());
    }
    nonempty("measurement method", &measurement.method)?;
    nonempty("measurement method_version", &measurement.method_version)?;
    let interval = &measurement.interval;
    if !measurement.estimate.is_finite()
        || !interval.lower.is_finite()
        || !interval.upper.is_finite()
        || !interval.confidence.is_finite()
    {
        return Err("measurement values must be finite".to_string());
    }
    if interval.lower > measurement.estimate || measurement.estimate > interval.upper {
        return Err("measurement estimate must lie inside its interval".to_string());
    }
    if interval.confidence <= 0.0 || interval.confidence >= 1.0 {
        return Err("measurement confidence must be in (0, 1)".to_string());
    }
    if measurement
        .power
        .is_some_and(|power| !power.is_finite() || !(0.0..=1.0).contains(&power))
    {
        return Err("measurement power must be finite and in [0, 1]".to_string());
    }
    Ok(())
}

fn validate_offline_poc_controls(controls: &OfflinePocControls) -> Result<(), String> {
    validate_digest(
        "offline PoC positive control",
        &controls.positive_control_sha256,
    )?;
    validate_digest(
        "offline PoC negative control",
        &controls.negative_control_sha256,
    )?;
    if controls.positive_control_sha256 == controls.negative_control_sha256 {
        return Err("offline PoC controls must produce distinct receipts".to_string());
    }
    if !controls.positive_passed || !controls.negative_passed {
        return Err("offline PoC requires passing positive and negative controls".to_string());
    }
    Ok(())
}
