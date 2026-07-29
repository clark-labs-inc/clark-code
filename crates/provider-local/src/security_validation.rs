use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::identity::{candidate_fingerprint, hex_digest};
use super::{
    SealedSecurityFinding, SecurityAttackPath, SecurityCandidate, SecurityCoverage,
    SecurityCoverageStatus, SecurityDeepLedger, SecurityDiffInventory, SecurityDisposition,
    SecurityInventory, SecurityLocation, SecurityPocLedger, SecurityPocOutcome, SecurityScanBundle,
    SecurityScanMode, SecurityScanPhase, SecurityScanSeal, SecurityThreatModel,
    SECURITY_SCAN_CONTRACT_VERSION,
};

pub fn finalize_security_scan(
    bundle: &SecurityScanBundle,
    inventory: &SecurityInventory,
    poc_ledger: &SecurityPocLedger,
) -> Result<SecurityScanSeal, String> {
    validate_header(bundle, inventory, SecurityScanMode::Standard)?;
    if bundle.diff_target.is_some()
        || bundle.deep_run_id.is_some()
        || !bundle.supporting_coverage.is_empty()
    {
        return Err(
            "standard scans must not contain diffTarget, deepRunId, or supportingCoverage".into(),
        );
    }
    validate_threat_model(&bundle.threat_model)?;
    let expected = inventory.paths.iter().cloned().collect::<BTreeSet<_>>();
    let (reviewed_files, excluded_files) =
        validate_coverage_rows(&bundle.coverage, &expected, "target inventory")?;
    let allowed = expected.iter().map(String::as_str).collect::<HashSet<_>>();
    let findings = validate_candidates(bundle, &allowed, None, poc_ledger)?;
    seal(
        bundle,
        inventory,
        SealMetadata {
            reviewed_files,
            excluded_files,
            ..SealMetadata::default()
        },
        findings,
    )
}

pub fn finalize_security_diff(
    bundle: &SecurityScanBundle,
    inventory: &SecurityInventory,
    diff: &SecurityDiffInventory,
    poc_ledger: &SecurityPocLedger,
) -> Result<SecurityScanSeal, String> {
    validate_header(bundle, inventory, SecurityScanMode::Diff)?;
    if bundle.deep_run_id.is_some() {
        return Err("diff scans must not contain deepRunId".into());
    }
    if diff.scope != inventory.scope {
        return Err("diff scope does not match the repository inventory scope".into());
    }
    let target = bundle
        .diff_target
        .as_ref()
        .ok_or_else(|| "diff scans require diffTarget from diff_inventory".to_string())?;
    if target != &diff.target {
        return Err("diffTarget is stale or belongs to a different Git target".into());
    }
    validate_threat_model(&bundle.threat_model)?;

    let changed = diff
        .changed_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    let (reviewed_files, excluded_files) =
        validate_coverage_rows(&bundle.coverage, &changed, "changed-file inventory")?;
    let repository_paths = inventory.paths.iter().cloned().collect::<BTreeSet<_>>();
    let supporting =
        validate_supporting_coverage(&bundle.supporting_coverage, &repository_paths, &changed)?;

    let mut allowed = repository_paths.clone();
    let mut changed_evidence = changed;
    for file in &diff.changed_files {
        if let Some(previous) = &file.previous_path {
            allowed.insert(previous.clone());
            changed_evidence.insert(previous.clone());
        }
    }
    let allowed = allowed.iter().map(String::as_str).collect::<HashSet<_>>();
    let findings = validate_candidates(bundle, &allowed, Some(&changed_evidence), poc_ledger)?;
    seal(
        bundle,
        inventory,
        SealMetadata {
            diff_target_id: Some(diff.target.target_id.clone()),
            reviewed_files,
            excluded_files,
            supporting_files: supporting,
            ..SealMetadata::default()
        },
        findings,
    )
}

pub(crate) fn finalize_security_deep(
    bundle: &SecurityScanBundle,
    inventory: &SecurityInventory,
    ledger: &SecurityDeepLedger,
    poc_ledger: &SecurityPocLedger,
) -> Result<SecurityScanSeal, String> {
    validate_header(bundle, inventory, SecurityScanMode::Deep)?;
    if bundle.diff_target.is_some() || !bundle.supporting_coverage.is_empty() {
        return Err("deep scans must not contain diffTarget or supportingCoverage".into());
    }
    let run_id = bundle
        .deep_run_id
        .as_deref()
        .ok_or_else(|| "deep scans require deepRunId from deep_begin".to_string())?;
    validate_threat_model(&bundle.threat_model)?;
    let expected = inventory.paths.iter().cloned().collect::<BTreeSet<_>>();
    let (reviewed_files, excluded_files) =
        validate_coverage_rows(&bundle.coverage, &expected, "target inventory")?;
    let allowed = expected.iter().map(String::as_str).collect::<HashSet<_>>();
    let findings = validate_candidates(bundle, &allowed, None, poc_ledger)?;
    let candidate_ids = bundle
        .candidates
        .iter()
        .map(|candidate| candidate.candidate_id.clone())
        .collect::<BTreeSet<_>>();
    let pass_count = ledger.validate(
        run_id,
        &bundle.scan_id,
        &inventory.inventory_id,
        &candidate_ids,
    )?;
    seal(
        bundle,
        inventory,
        SealMetadata {
            deep_run_id: Some(run_id.to_string()),
            deep_passes: Some(pass_count),
            reviewed_files,
            excluded_files,
            ..SealMetadata::default()
        },
        findings,
    )
}

#[derive(Default)]
struct SealMetadata {
    diff_target_id: Option<String>,
    deep_run_id: Option<String>,
    deep_passes: Option<usize>,
    reviewed_files: usize,
    excluded_files: usize,
    supporting_files: usize,
}

fn seal(
    bundle: &SecurityScanBundle,
    inventory: &SecurityInventory,
    metadata: SealMetadata,
    findings: Vec<SealedSecurityFinding>,
) -> Result<SecurityScanSeal, String> {
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
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|error| format!("cannot canonicalize security bundle: {error}"))?;
    Ok(SecurityScanSeal {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: bundle.scan_id.clone(),
        model: bundle.model.clone(),
        scope: bundle.scope.clone(),
        inventory_id: inventory.inventory_id.clone(),
        diff_target_id: metadata.diff_target_id,
        deep_run_id: metadata.deep_run_id,
        deep_passes: metadata.deep_passes,
        bundle_digest: hex_digest(&encoded),
        reviewed_files: metadata.reviewed_files,
        excluded_files: metadata.excluded_files,
        supporting_files: metadata.supporting_files,
        candidate_count: bundle.candidates.len(),
        poc_attempted_count: bundle.candidates.len(),
        poc_reproduced_count: bundle
            .candidates
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.poc.outcome,
                    SecurityPocOutcome::Reproduced | SecurityPocOutcome::PartiallyReproduced
                )
            })
            .count(),
        findings,
    })
}

fn validate_header(
    bundle: &SecurityScanBundle,
    inventory: &SecurityInventory,
    mode: SecurityScanMode,
) -> Result<(), String> {
    if bundle.contract_version != SECURITY_SCAN_CONTRACT_VERSION {
        return Err(format!(
            "unsupported security contract version {}",
            bundle.contract_version
        ));
    }
    require_text("scanId", &bundle.scan_id)?;
    require_text("model", &bundle.model)?;
    if bundle.scope != inventory.scope {
        return Err(format!(
            "bundle scope `{}` does not match inventoried scope `{}`",
            bundle.scope, inventory.scope
        ));
    }
    if bundle.inventory_id != inventory.inventory_id {
        return Err("inventoryId is stale or belongs to a different target snapshot".into());
    }
    if bundle.phase != SecurityScanPhase::Reporting {
        return Err("scan cannot finalize before the reporting phase".into());
    }
    if bundle.mode != mode {
        return Err(format!(
            "expected `{mode:?}` security mode, received `{:?}`",
            bundle.mode
        ));
    }
    Ok(())
}

fn validate_threat_model(model: &SecurityThreatModel) -> Result<(), String> {
    for (name, values) in [
        ("assets", &model.assets),
        ("trustBoundaries", &model.trust_boundaries),
        ("attackerInputs", &model.attacker_inputs),
        ("invariants", &model.invariants),
    ] {
        if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
            return Err(format!("threatModel.{name} must contain grounded entries"));
        }
    }
    Ok(())
}

fn validate_coverage_rows(
    rows: &[SecurityCoverage],
    expected: &BTreeSet<String>,
    target_name: &str,
) -> Result<(usize, usize), String> {
    let mut observed = BTreeMap::new();
    let mut reviewed = 0usize;
    let mut excluded = 0usize;
    for entry in rows {
        if observed.insert(entry.path.clone(), entry).is_some() {
            return Err(format!("duplicate coverage row for `{}`", entry.path));
        }
        match entry.status {
            SecurityCoverageStatus::Reviewed => reviewed += 1,
            SecurityCoverageStatus::Excluded => {
                excluded += 1;
                require_exclusion_reason(entry)?;
            }
        }
    }
    let actual = observed.keys().cloned().collect::<BTreeSet<_>>();
    let missing = expected
        .difference(&actual)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    let extra = actual
        .difference(expected)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(format!(
            "coverage does not match {target_name}; missing={missing:?}, extra={extra:?}"
        ));
    }
    Ok((reviewed, excluded))
}

fn validate_supporting_coverage(
    rows: &[SecurityCoverage],
    repository_paths: &BTreeSet<String>,
    changed: &BTreeSet<String>,
) -> Result<usize, String> {
    let mut observed = HashSet::new();
    for entry in rows {
        if !observed.insert(entry.path.as_str()) {
            return Err(format!(
                "duplicate supporting coverage row for `{}`",
                entry.path
            ));
        }
        if changed.contains(&entry.path) {
            return Err(format!(
                "changed path `{}` belongs in coverage, not supportingCoverage",
                entry.path
            ));
        }
        if !repository_paths.contains(&entry.path) {
            return Err(format!(
                "supporting coverage path `{}` is not in the repository inventory",
                entry.path
            ));
        }
        if entry.status == SecurityCoverageStatus::Excluded {
            require_exclusion_reason(entry)?;
        }
    }
    Ok(rows.len())
}

fn require_exclusion_reason(entry: &SecurityCoverage) -> Result<(), String> {
    if entry
        .reason
        .as_deref()
        .is_none_or(|reason| reason.trim().is_empty())
    {
        Err(format!(
            "excluded coverage row `{}` requires a reason",
            entry.path
        ))
    } else {
        Ok(())
    }
}

fn validate_candidates(
    bundle: &SecurityScanBundle,
    paths: &HashSet<&str>,
    changed_paths: Option<&BTreeSet<String>>,
    poc_ledger: &SecurityPocLedger,
) -> Result<Vec<SealedSecurityFinding>, String> {
    let mut candidate_ids = HashSet::new();
    let mut semantic_identities = HashSet::new();
    let mut findings = Vec::new();
    for candidate in &bundle.candidates {
        require_text("candidateId", &candidate.candidate_id)?;
        if !candidate_ids.insert(candidate.candidate_id.as_str()) {
            return Err(format!(
                "duplicate candidateId `{}`",
                candidate.candidate_id
            ));
        }
        validate_candidate_metadata(candidate)?;
        let fingerprint = candidate_fingerprint(candidate);
        if !semantic_identities.insert(fingerprint.clone()) {
            return Err(format!(
                "multiple candidates resolve to the same Clark Security identity `{}`",
                candidate.identity_anchor
            ));
        }
        validate_location("source", &candidate.source, paths)?;
        validate_location("control", &candidate.control, paths)?;
        validate_location("sink", &candidate.sink, paths)?;
        if changed_paths.is_some_and(|changed| {
            ![
                candidate.source.path.as_str(),
                candidate.control.path.as_str(),
                candidate.sink.path.as_str(),
            ]
            .iter()
            .any(|path| changed.contains(*path))
        }) {
            return Err(format!(
                "diff candidate `{}` does not touch a changed path",
                candidate.candidate_id
            ));
        }
        require_text("impact", &candidate.impact)?;
        require_text("validation.evidence", &candidate.validation.evidence)?;
        let (positive_receipt, negative_receipt) =
            poc_ledger.validate_candidate(&bundle.scan_id, &bundle.inventory_id, candidate)?;
        if candidate.validation.disposition == SecurityDisposition::Reportable {
            let attack = candidate.attack_path.as_ref().ok_or_else(|| {
                format!(
                    "reportable candidate `{}` has no attackPath",
                    candidate.candidate_id
                )
            })?;
            validate_attack_path(attack)?;
            findings.push(SealedSecurityFinding {
                finding_id: format!("SEC-{}", &fingerprint[..16]),
                fingerprint,
                candidate_id: candidate.candidate_id.clone(),
                severity: candidate.severity,
                source_path: candidate.source.path.clone(),
                impact: candidate.impact.clone(),
                poc_outcome: candidate.poc.outcome,
                positive_receipt_id: positive_receipt
                    .expect("reportable PoC validation requires a positive receipt")
                    .receipt_id
                    .clone(),
                negative_receipt_id: negative_receipt
                    .expect("reportable PoC validation requires a negative receipt")
                    .receipt_id
                    .clone(),
            });
        }
    }
    findings.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    Ok(findings)
}

fn validate_candidate_metadata(candidate: &SecurityCandidate) -> Result<(), String> {
    validate_identity_slug("ruleId", &candidate.rule_id, 256)?;
    validate_identity_slug("identityAnchor", &candidate.identity_anchor, 512)?;
    if let Some(instance) = &candidate.identity_instance {
        validate_identity_slug("identityInstance", instance, 512)?;
    }
    require_text("title", &candidate.title)?;
    require_text("summary", &candidate.summary)?;
    validate_identity_slug("category", &candidate.category, 256)?;
    if candidate.cwe.len() > 64 {
        return Err("cwe must contain no more than 64 taxonomy identifiers".into());
    }
    for cwe in &candidate.cwe {
        let digits = cwe.strip_prefix("CWE-").unwrap_or_default();
        if digits.is_empty()
            || digits.len() > 6
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!("invalid CWE taxonomy identifier `{cwe}`"));
        }
    }
    require_text("remediation", &candidate.remediation)
}

fn validate_identity_slug(name: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_len
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(format!(
            "{name} must be a lowercase stable slug of at most {max_len} bytes"
        ));
    }
    Ok(())
}

fn validate_location(
    name: &str,
    location: &SecurityLocation,
    paths: &HashSet<&str>,
) -> Result<(), String> {
    if !paths.contains(location.path.as_str()) {
        return Err(format!(
            "candidate {name} path `{}` is not in the target evidence set",
            location.path
        ));
    }
    if location.line.is_none_or(|line| line == 0) {
        return Err(format!(
            "candidate {name} location requires a concrete one-based line"
        ));
    }
    require_text(&format!("{name}.description"), &location.description)
}

fn validate_attack_path(path: &SecurityAttackPath) -> Result<(), String> {
    require_text("attackPath.attacker", &path.attacker)?;
    require_text("attackPath.entrypoint", &path.entrypoint)?;
    require_text("attackPath.likelihood", &path.likelihood)?;
    if path.path.is_empty() || path.path.iter().any(|step| step.trim().is_empty()) {
        return Err("attackPath.path must contain concrete reachability steps".into());
    }
    Ok(())
}

fn require_text(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}
