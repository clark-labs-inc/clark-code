use std::fmt::Write;

use super::{
    ClaimRecord, ClaimStatus, EvidenceProducer, EvidenceRecord, ScoutSnapshot, VerificationOutcome,
};

pub(super) fn render(snapshot: &ScoutSnapshot, fingerprint: Option<&str>) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "# Scout report");
    let _ = writeln!(output);

    let adjudicated = snapshot
        .claims
        .values()
        .filter(|claim| claim.status.is_adjudicated())
        .count();
    let supported = snapshot
        .claims
        .values()
        .filter(|claim| claim.status == ClaimStatus::Supported)
        .count();
    let _ = writeln!(output, "## TL;DR");
    let _ = writeln!(
        output,
        "{} claims, {} adjudicated, {} supported, {} evidence artifacts, {} events.",
        snapshot.claims.len(),
        adjudicated,
        supported,
        snapshot.evidence.len(),
        snapshot.event_count
    );
    if snapshot.disposition.is_none() {
        let _ = writeln!(output, "This run is not sealed.");
    }

    let _ = writeln!(output);
    let _ = writeln!(output, "## Charter");
    let _ = writeln!(output, "- Run: `{}`", snapshot.charter.run_id);
    let _ = writeln!(output, "- Objective: {}", snapshot.charter.objective);
    let _ = writeln!(output, "- Snapshot: `{}`", snapshot.charter.snapshot_id);
    let _ = writeln!(
        output,
        "- Capability census: `{}` (`{}`)",
        snapshot.charter.capability_census_id, snapshot.charter.capability_fingerprint
    );
    let _ = writeln!(
        output,
        "- Scopes: {}",
        join_strings(snapshot.charter.scopes.iter().map(String::as_str))
    );
    let _ = writeln!(
        output,
        "- Exclusions: {}",
        join_strings(snapshot.charter.exclusions.iter().map(String::as_str))
    );
    let _ = writeln!(
        output,
        "- Production read-only: `{}`; network allowed: `{}`",
        snapshot.charter.capabilities.production_read_only,
        snapshot.charter.capabilities.network_allowed
    );
    let _ = writeln!(
        output,
        "- Denied capabilities: {}",
        join_strings(
            snapshot
                .charter
                .capabilities
                .denied
                .iter()
                .map(String::as_str)
        )
    );
    let _ = writeln!(
        output,
        "- Limits: {} parallel agents, {} worker submissions, {} claims, {} artifacts, {} events",
        snapshot.charter.limits.max_parallel_agents,
        snapshot.charter.limits.max_worker_submissions,
        snapshot.charter.limits.max_claims,
        snapshot.charter.limits.max_artifacts,
        snapshot.charter.limits.max_events
    );
    let _ = writeln!(
        output,
        "- Minimum quantitative power: `{:.3}`",
        snapshot.charter.minimum_power
    );
    let _ = writeln!(output, "- Phase: `{:?}`", snapshot.phase);
    if let Some(disposition) = snapshot.disposition {
        let _ = writeln!(output, "- Disposition: `{:?}`", disposition);
    }
    if let Some(fingerprint) = fingerprint {
        let _ = writeln!(output, "- Ledger SHA-256: `{fingerprint}`");
    }

    let _ = writeln!(output);
    let _ = writeln!(output, "## Claim ledger");
    if snapshot.claims.is_empty() {
        let _ = writeln!(output, "- None.");
    } else {
        for claim in snapshot.claims.values() {
            render_claim(&mut output, claim);
        }
    }

    let _ = writeln!(output);
    let _ = writeln!(output, "## Evidence");
    for evidence in snapshot.evidence.values() {
        render_evidence(&mut output, evidence);
    }
    if snapshot.evidence.is_empty() {
        let _ = writeln!(output, "- None.");
    }

    let _ = writeln!(output);
    let _ = writeln!(output, "## Coverage and limitations");
    for coverage in &snapshot.coverage {
        let _ = writeln!(output, "- Covered: {coverage}");
    }
    for limitation in &snapshot.limitations {
        let _ = writeln!(output, "- Limitation: {limitation}");
    }
    for followup in &snapshot.requested_followups {
        let _ = writeln!(output, "- Follow-up: {followup}");
    }
    if snapshot.coverage.is_empty()
        && snapshot.limitations.is_empty()
        && snapshot.requested_followups.is_empty()
    {
        let _ = writeln!(output, "- None recorded.");
    }
    output
}

fn render_claim(output: &mut String, claim: &ClaimRecord) {
    let _ = writeln!(
        output,
        "- **{}** `{}` — {}",
        verdict_label(claim.status),
        claim.proposal.id,
        claim.proposal.text
    );
    let _ = writeln!(
        output,
        "  Origin: `{}`; headline: `{}`; quantitative: `{}`",
        claim.originating_assignment, claim.proposal.headline, claim.proposal.quantitative
    );
    if let Some(required) = claim.proposal.required_tier {
        let _ = writeln!(output, "  Required tier: `{required:?}`");
    }
    let _ = writeln!(
        output,
        "  Evidence: {}; counterevidence: {}",
        join_display(&claim.proposal.evidence),
        join_display(&claim.proposal.counterevidence)
    );
    if !claim.proposal.assumptions.is_empty() {
        let _ = writeln!(
            output,
            "  Assumptions: {}",
            join_strings(claim.proposal.assumptions.iter().map(String::as_str))
        );
    }
    if let Some(instrument) = &claim.proposal.missing_instrument {
        let _ = writeln!(output, "  Missing instrument: {instrument}");
    }
    if let Some(adjudication) = &claim.adjudication {
        let _ = writeln!(
            output,
            "  Test: {}. Reason: {}",
            adjudication.test, adjudication.reason
        );
        if let Some(tier) = adjudication.proof_tier {
            let _ = writeln!(output, "  Adjudicated tier: `{tier:?}`");
        }
        if let Some(instrument) = &adjudication.instrument_needed {
            let _ = writeln!(output, "  Instrument needed: {instrument}");
        }
    }
    if let Some(reason) = &claim.retraction_reason {
        let _ = writeln!(output, "  Retraction reason: {reason}");
    }
    if let Some(reason) = &claim.supersession_reason {
        let replacement = claim
            .superseded_by
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".into());
        let _ = writeln!(
            output,
            "  Supersession reason: {reason}; replacement: `{replacement}`"
        );
    }
    for limitation in &claim.limitations {
        let _ = writeln!(output, "  Limitation: {limitation}");
    }
}

fn render_evidence(output: &mut String, evidence: &EvidenceRecord) {
    let verification = match evidence.checks.last() {
        Some(check) => format!("{:?} by {}", check.outcome, check.verifier),
        None if matches!(evidence.producer, EvidenceProducer::Runner { .. }) => {
            "host-recorded".into()
        }
        None => "unverified worker artifact".into(),
    };
    let producer = match &evidence.producer {
        EvidenceProducer::Worker {
            assignment_id,
            role,
        } => format!("worker {assignment_id} ({role:?})"),
        EvidenceProducer::Runner { runner_id } => format!("runner {runner_id}"),
    };
    let _ = writeln!(
        output,
        "- `{}` **{:?}** — {}",
        evidence.artifact.id, evidence.artifact.kind, evidence.artifact.source
    );
    let _ = writeln!(
        output,
        "  SHA-256: `{}`; producer: {}; verification: {}",
        evidence.artifact.content_sha256, producer, verification
    );
    if let Some(tier) = evidence.artifact.proof_tier {
        let _ = writeln!(output, "  Declared tier: `{tier:?}`");
    }
    if let Some(measurement) = &evidence.artifact.measurement {
        let power = measurement
            .power
            .map(|power| format!("{power:.6}"))
            .unwrap_or_else(|| "not computed".into());
        let _ = writeln!(
            output,
            "  Measurement: n={}, missing={}, estimate={:.6}, {:.1}% CI [{:.6}, {:.6}], method={}@{}, power={}",
            measurement.sample_size,
            measurement.missing,
            measurement.estimate,
            measurement.interval.confidence * 100.0,
            measurement.interval.lower,
            measurement.interval.upper,
            measurement.method,
            measurement.method_version,
            power
        );
    }
    if let Some(controls) = &evidence.artifact.offline_poc_controls {
        let _ = writeln!(
            output,
            "  Controls: positive=`{}` passed=`{}`; negative=`{}` passed=`{}`",
            controls.positive_control_sha256,
            controls.positive_passed,
            controls.negative_control_sha256,
            controls.negative_passed
        );
    }
    if let Some(target) = &evidence.artifact.reproduces {
        let _ = writeln!(output, "  Reproduces: `{target}`");
    }
    if evidence.artifact.recipe.is_some() {
        let _ = writeln!(
            output,
            "  Replay recipe: retained in the append-only ledger (not expanded in the report)."
        );
    }
    if let Some(check) = evidence.checks.last() {
        let label = match check.outcome {
            VerificationOutcome::Exact => "exact",
            VerificationOutcome::Equivalent => "equivalent",
            VerificationOutcome::Changed => "changed",
            VerificationOutcome::Unavailable => "unavailable",
            VerificationOutcome::Failed => "failed",
        };
        let _ = writeln!(
            output,
            "  Latest check: {label} at {} ms — {}",
            check.checked_at_ms, check.reason
        );
    }
}

fn join_display<T: std::fmt::Display + Ord>(values: &std::collections::BTreeSet<T>) -> String {
    join_strings(values.iter().map(ToString::to_string))
}

fn join_strings(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let values = values
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect::<Vec<_>>();
    if values.is_empty() {
        "none".into()
    } else {
        values.join(", ")
    }
}

fn verdict_label(status: ClaimStatus) -> &'static str {
    match status {
        ClaimStatus::Proposed => "PROPOSED",
        ClaimStatus::Checked => "CHECKED",
        ClaimStatus::Supported => "SUPPORTED",
        ClaimStatus::Unsupported => "UNSUPPORTED",
        ClaimStatus::Unfalsifiable => "UNFALSIFIABLE",
        ClaimStatus::Retracted => "RETRACTED",
        ClaimStatus::Superseded => "SUPERSEDED",
    }
}
