use std::collections::BTreeSet;

use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::security::{
    SecurityCandidate, SecurityCoverageStatus, SecurityDisposition, SecurityScanBundle,
    SecurityScanMode,
};
use crate::security_history::SecurityScanRecord;

#[path = "security_export_model.rs"]
mod model;
pub use model::{
    ClarkSecurityCloudExport, ClarkSecurityCloudIdentity, ClarkSecurityCoverageSurfaceDraft,
    ClarkSecurityFindingIdentityDraft, ClarkSecurityLocationDraft, ClarkSecurityOccurrenceDraft,
};

const DOCUMENT_SCHEMA_VERSION: &str = "1.0";
const FINGERPRINT_ALGORITHM: &str = "clark-security/v2";

pub fn clark_security_cloud_identity(
    record: &SecurityScanRecord,
) -> Result<ClarkSecurityCloudIdentity, String> {
    let seal = verified_local_seal(record)?;
    require_lower_hex("inventoryId", &record.bundle.inventory_id)?;
    require_lower_hex("bundleDigest", &seal.bundle_digest)?;
    Ok(ClarkSecurityCloudIdentity {
        client_scan_id: format!("scan:desktop:{}", &seal.bundle_digest[..32]),
        idempotency_key: format!("clark-security-desktop:{}", seal.bundle_digest),
        inventory_id: format!("inventory:{}", record.bundle.inventory_id),
        snapshot_digest: format!("sha256:{}", record.bundle.inventory_id),
        mode: record.bundle.mode,
    })
}

pub fn build_clark_security_cloud_export(
    record: &SecurityScanRecord,
    repository_id: Uuid,
    platform_scan_id: Uuid,
) -> Result<ClarkSecurityCloudExport, String> {
    let seal = verified_local_seal(record)?;
    let identity = clark_security_cloud_identity(record)?;
    let candidates = record
        .bundle
        .candidates
        .iter()
        .map(|candidate| occurrence_draft(record, candidate))
        .collect::<Result<Vec<_>, _>>()?;
    let sealed_candidates = seal
        .findings
        .iter()
        .map(|finding| finding.candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    let reportable_candidates = record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| candidate.validation.disposition == SecurityDisposition::Reportable)
        .map(|candidate| candidate.candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    if sealed_candidates != reportable_candidates {
        return Err("local Clark Security seal does not match reportable candidates".into());
    }

    let mut surfaces = vec![scope_surface(&record.bundle)];
    surfaces.extend(record.bundle.candidates.iter().map(candidate_surface));
    let has_deferred = record
        .bundle
        .candidates
        .iter()
        .any(|candidate| candidate.validation.disposition == SecurityDisposition::Deferred);
    let completeness = if has_deferred { "partial" } else { "complete" };
    let findings = record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| candidate.validation.disposition == SecurityDisposition::Reportable)
        .map(|candidate| canonical_finding(record, candidate, repository_id, platform_scan_id))
        .collect::<Result<Vec<_>, _>>()?;
    let coverage = canonical_coverage(record, completeness, &surfaces, &identity.client_scan_id);
    let manifest = json!({
        "documentType": "clark-security.scan-manifest",
        "schemaVersion": DOCUMENT_SCHEMA_VERSION,
        "scan": {
            "id": identity.client_scan_id,
            "status": "completed",
            "target": {
                "repositoryId": repository_id,
                "snapshotDigest": identity.snapshot_digest,
            },
            "coverageRef": "coverage.json",
            "findingsRef": "findings.json",
        }
    });
    let findings = json!({
        "documentType": "clark-security.findings",
        "schemaVersion": DOCUMENT_SCHEMA_VERSION,
        "scanId": identity.client_scan_id,
        "findings": findings,
    });
    Ok(ClarkSecurityCloudExport {
        identity,
        manifest,
        findings,
        coverage,
        coverage_completeness: completeness.into(),
        coverage_surfaces: surfaces,
        occurrences: candidates,
    })
}

fn verified_local_seal(
    record: &SecurityScanRecord,
) -> Result<&crate::security::SecurityScanSeal, String> {
    let seal = record
        .seal
        .as_ref()
        .ok_or_else(|| "only locally sealed Clark Security scans can sync".to_string())?;
    if seal.scan_id != record.bundle.scan_id
        || seal.inventory_id != record.bundle.inventory_id
        || seal.bundle_digest != bundle_digest(&record.bundle)?
    {
        return Err("local Clark Security bundle or seal is stale or has been modified".into());
    }
    Ok(seal)
}

fn bundle_digest(bundle: &SecurityScanBundle) -> Result<String, String> {
    let mut canonical = bundle.clone();
    canonical
        .coverage
        .sort_by(|left, right| left.path.cmp(&right.path));
    canonical
        .supporting_coverage
        .sort_by(|left, right| left.path.cmp(&right.path));
    canonical
        .candidates
        .sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("cannot encode Clark Security bundle: {error}"))?;
    Ok(sha256_hex(&bytes))
}

fn occurrence_draft(
    record: &SecurityScanRecord,
    candidate: &SecurityCandidate,
) -> Result<ClarkSecurityOccurrenceDraft, String> {
    let candidate_id = wire_candidate_id(&candidate.candidate_id);
    let surface_id = candidate_surface_id(candidate);
    let attack_path = candidate
        .attack_path
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("cannot encode attack path: {error}"))?
        .unwrap_or_else(|| {
            json!({
                "status": occurrence_disposition(candidate.validation.disposition),
                "proofGaps": candidate.poc.limitations,
            })
        });
    Ok(ClarkSecurityOccurrenceDraft {
        candidate_id,
        identity: ClarkSecurityFindingIdentityDraft {
            rule_id: candidate.rule_id.clone(),
            anchor: candidate.identity_anchor.clone(),
            instance: candidate.identity_instance.clone(),
        },
        disposition: occurrence_disposition(candidate.validation.disposition).into(),
        severity: candidate.severity,
        confidence: candidate.confidence,
        title: candidate.title.clone(),
        summary: candidate.summary.clone(),
        category: candidate.category.clone(),
        cwe: candidate.cwe.clone(),
        root_cause: candidate_root_cause(candidate),
        attack_path,
        remediation: candidate.remediation.clone(),
        locations: candidate_locations(candidate)?,
        provenance: candidate_provenance(record),
        coverage_surface_ids: vec![surface_id],
        poc_outcome: candidate.poc.outcome,
        local_poc_receipt_ids: [
            candidate.poc.positive_receipt_id.clone(),
            candidate.poc.negative_receipt_id.clone(),
        ]
        .into_iter()
        .flatten()
        .collect(),
    })
}

fn canonical_finding(
    record: &SecurityScanRecord,
    candidate: &SecurityCandidate,
    repository_id: Uuid,
    platform_scan_id: Uuid,
) -> Result<Value, String> {
    let (finding_id, occurrence_id, fingerprint) =
        derive_identity(repository_id, platform_scan_id, candidate);
    let attack_path = candidate.attack_path.as_ref().ok_or_else(|| {
        format!(
            "reportable candidate {} has no attack path",
            candidate.candidate_id
        )
    })?;
    Ok(json!({
        "findingId": finding_id,
        "occurrenceId": occurrence_id,
        "ruleId": candidate.rule_id,
        "identity": {
            "anchor": candidate.identity_anchor,
            "instance": candidate.identity_instance,
        },
        "fingerprints": {
            "algorithm": FINGERPRINT_ALGORITHM,
            "primary": fingerprint,
        },
        "title": candidate.title,
        "summary": candidate.summary,
        "severity": {"level": candidate.severity},
        "confidence": {
            "level": candidate.confidence,
            "rationale": candidate.validation.evidence,
        },
        "taxonomy": {
            "category": candidate.category,
            "cwe": candidate.cwe,
        },
        "locations": candidate_locations(candidate)?,
        "rootCause": candidate_root_cause(candidate),
        "attackPath": attack_path,
        "remediation": candidate.remediation,
        "provenance": candidate_provenance(record),
        "extensions": {"candidateId": wire_candidate_id(&candidate.candidate_id)},
    }))
}

fn candidate_root_cause(candidate: &SecurityCandidate) -> Value {
    json!({
        "summary": candidate.validation.evidence,
        "counterevidence": candidate.validation.counterevidence,
        "source": candidate.source,
        "control": candidate.control,
        "sink": candidate.sink,
        "impact": candidate.impact,
    })
}

fn candidate_provenance(record: &SecurityScanRecord) -> Value {
    json!({
        "source": "clark-security",
        "contractVersion": record.bundle.contract_version,
        "localScanId": record.bundle.scan_id,
        "localSealDigest": record.seal.as_ref().map(|seal| &seal.bundle_digest),
    })
}

fn canonical_coverage(
    record: &SecurityScanRecord,
    completeness: &str,
    surfaces: &[ClarkSecurityCoverageSurfaceDraft],
    client_scan_id: &str,
) -> Value {
    let reviewed = record
        .bundle
        .coverage
        .iter()
        .filter(|row| row.status == SecurityCoverageStatus::Reviewed)
        .map(|row| row.path.clone())
        .collect::<Vec<_>>();
    let exclusions = record
        .bundle
        .coverage
        .iter()
        .filter(|row| row.status == SecurityCoverageStatus::Excluded)
        .map(|row| {
            json!({
                "path": row.path,
                "reason": row.reason,
            })
        })
        .collect::<Vec<_>>();
    let deferred = record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| candidate.validation.disposition == SecurityDisposition::Deferred)
        .map(|candidate| {
            json!({
                "id": wire_candidate_id(&candidate.candidate_id),
                "surfaceIds": [candidate_surface_id(candidate)],
                "reason": candidate.poc.limitations.join("; "),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "documentType": "clark-security.coverage",
        "schemaVersion": DOCUMENT_SCHEMA_VERSION,
        "scanId": client_scan_id,
        "mode": coverage_mode(record.bundle.mode),
        "completeness": completeness,
        "inventoryStrategy": "clark_repository_inventory",
        "includePaths": reviewed,
        "excludePaths": exclusions.iter().filter_map(|row| {
            row.get("path").and_then(Value::as_str)
        }).collect::<Vec<_>>(),
        "surfaces": surfaces.iter().map(|surface| json!({
            "id": surface.surface_id,
            "label": surface.label,
            "disposition": surface.disposition,
            "receiptRefs": [],
            "riskArea": surface.risk_area,
            "notes": surface.notes,
        })).collect::<Vec<_>>(),
        "explicitExclusions": exclusions,
        "deferred": deferred,
    })
}

fn scope_surface(bundle: &SecurityScanBundle) -> ClarkSecurityCoverageSurfaceDraft {
    let reviewed = bundle
        .coverage
        .iter()
        .filter(|row| row.status == SecurityCoverageStatus::Reviewed)
        .count();
    let excluded = bundle.coverage.len().saturating_sub(reviewed);
    ClarkSecurityCoverageSurfaceDraft {
        surface_id: format!("scope:{}", &bundle.inventory_id[..24]),
        label: format!("Clark Security scope {}", bundle.scope),
        disposition: "no_issue_found".into(),
        risk_area: Some("repository-scope".into()),
        notes: Some(format!(
            "{reviewed} files reviewed and {excluded} files explicitly excluded"
        )),
    }
}

fn candidate_surface(candidate: &SecurityCandidate) -> ClarkSecurityCoverageSurfaceDraft {
    ClarkSecurityCoverageSurfaceDraft {
        surface_id: candidate_surface_id(candidate),
        label: candidate.title.clone(),
        disposition: surface_disposition(candidate.validation.disposition).into(),
        risk_area: Some(candidate.category.clone()),
        notes: Some(candidate.validation.evidence.clone()),
    }
}

fn candidate_locations(
    candidate: &SecurityCandidate,
) -> Result<Vec<ClarkSecurityLocationDraft>, String> {
    [
        (&candidate.source, "entrypoint"),
        (&candidate.control, "root_control"),
        (&candidate.sink, "sink"),
    ]
    .into_iter()
    .map(|(location, role)| {
        let line = location.line.ok_or_else(|| {
            format!(
                "candidate {} has no concrete line for {role}",
                candidate.candidate_id
            )
        })?;
        Ok(ClarkSecurityLocationDraft {
            path: location.path.clone(),
            start_line: line,
            end_line: Some(line),
            role: role.into(),
        })
    })
    .collect()
}

fn derive_identity(
    repository_id: Uuid,
    scan_id: Uuid,
    candidate: &SecurityCandidate,
) -> (String, String, String) {
    let semantic = [
        FINGERPRINT_ALGORITHM,
        &repository_id.to_string(),
        &candidate.rule_id,
        &candidate.identity_anchor,
        candidate.identity_instance.as_deref().unwrap_or_default(),
    ]
    .join("\0");
    let fingerprint = format!("sha256:{}", sha256_hex(semantic.as_bytes()));
    let finding_input = format!("{FINGERPRINT_ALGORITHM}:{fingerprint}");
    let finding_id = format!("csf_{}", &sha256_hex(finding_input.as_bytes())[..24]);
    let occurrence_input = [scan_id.to_string(), finding_input].join("\0");
    let occurrence_id = format!("occ_{}", &sha256_hex(occurrence_input.as_bytes())[..24]);
    (finding_id, occurrence_id, fingerprint)
}

fn wire_candidate_id(local: &str) -> String {
    format!("candidate:{local}")
}

fn candidate_surface_id(candidate: &SecurityCandidate) -> String {
    let input = [
        candidate.rule_id.as_str(),
        candidate.identity_anchor.as_str(),
        candidate.identity_instance.as_deref().unwrap_or_default(),
    ]
    .join("\0");
    format!("candidate:{}", &sha256_hex(input.as_bytes())[..24])
}

fn occurrence_disposition(disposition: SecurityDisposition) -> &'static str {
    match disposition {
        SecurityDisposition::Reportable => "reported",
        SecurityDisposition::Suppressed => "rejected",
        SecurityDisposition::NotApplicable => "not_applicable",
        SecurityDisposition::Deferred => "deferred",
    }
}

fn surface_disposition(disposition: SecurityDisposition) -> &'static str {
    match disposition {
        SecurityDisposition::Reportable => "reported",
        SecurityDisposition::Suppressed => "rejected",
        SecurityDisposition::NotApplicable => "not_applicable",
        SecurityDisposition::Deferred => "needs_follow_up",
    }
}

fn coverage_mode(mode: SecurityScanMode) -> &'static str {
    match mode {
        SecurityScanMode::Standard => "repository",
        SecurityScanMode::Diff => "diff",
        SecurityScanMode::Deep => "deep_repository",
    }
}

fn require_lower_hex(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(format!("{label} must be a 64-character lowercase digest"));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "security_export_tests.rs"]
mod tests;
