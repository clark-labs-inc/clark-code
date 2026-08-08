//! Deterministic contracts for local agent security scans.
//!
//! The model performs semantic analysis. This module owns the parts that must
//! not depend on model judgment: target inventory, complete coverage, phase
//! closure, stable finding identities, and the final evidence seal.

use std::path::Path;

use crate::exec::Executor;
use serde::{Deserialize, Serialize};

#[path = "security_identity.rs"]
mod identity;
#[path = "security_deep.rs"]
mod security_deep;
#[path = "security_diff.rs"]
mod security_diff;
#[path = "security_poc.rs"]
mod security_poc;
#[path = "security_validation.rs"]
mod validation;

use identity::hex_digest;
#[cfg(test)]
use identity::inventory_digest;
use identity::inventory_snapshot_digest;
pub(crate) use security_deep::SecurityDeepLedger;
pub use security_deep::{SecurityDeepPassReceipt, SecurityDeepStatus, SecurityDeepTaskReceipt};
pub use security_diff::{
    collect_security_diff_inventory, SecurityDiffFile, SecurityDiffInventory, SecurityDiffKind,
    SecurityDiffTarget,
};
pub use security_poc::{
    SecurityPocControl, SecurityPocEvidence, SecurityPocExecutionMetadata, SecurityPocLedger,
    SecurityPocOutcome, SecurityPocReceipt,
};
pub(crate) use validation::finalize_security_deep;
pub use validation::{finalize_security_diff, finalize_security_scan};

pub const SECURITY_SCAN_CONTRACT_VERSION: u32 = 2;
const MAX_INVENTORY_FILES: usize = 100_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityInventory {
    pub contract_version: u32,
    pub scope: String,
    pub inventory_id: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityScanMode {
    Standard,
    Diff,
    Deep,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityScanPhase {
    Preflight,
    ThreatModel,
    Discovery,
    Validation,
    AttackPath,
    Reporting,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanBundle {
    pub contract_version: u32,
    pub scan_id: String,
    pub mode: SecurityScanMode,
    pub model: String,
    pub scope: String,
    pub inventory_id: String,
    pub phase: SecurityScanPhase,
    pub threat_model: SecurityThreatModel,
    pub coverage: Vec<SecurityCoverage>,
    #[serde(default)]
    pub supporting_coverage: Vec<SecurityCoverage>,
    #[serde(default)]
    pub diff_target: Option<SecurityDiffTarget>,
    #[serde(default)]
    pub deep_run_id: Option<String>,
    #[serde(default)]
    pub candidates: Vec<SecurityCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityThreatModel {
    pub assets: Vec<String>,
    pub trust_boundaries: Vec<String>,
    pub attacker_inputs: Vec<String>,
    pub invariants: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCoverageStatus {
    Reviewed,
    Excluded,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCoverage {
    pub path: String,
    pub status: SecurityCoverageStatus,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityLocation {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCandidate {
    pub candidate_id: String,
    pub rule_id: String,
    pub identity_anchor: String,
    #[serde(default)]
    pub identity_instance: Option<String>,
    pub title: String,
    pub summary: String,
    pub category: String,
    #[serde(default)]
    pub cwe: Vec<String>,
    pub severity: SecuritySeverity,
    pub confidence: SecurityConfidence,
    pub source: SecurityLocation,
    pub control: SecurityLocation,
    pub sink: SecurityLocation,
    pub impact: String,
    pub remediation: String,
    pub validation: SecurityValidation,
    pub poc: SecurityPocEvidence,
    #[serde(default)]
    pub attack_path: Option<SecurityAttackPath>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityDisposition {
    Reportable,
    Suppressed,
    NotApplicable,
    Deferred,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityValidation {
    pub disposition: SecurityDisposition,
    pub evidence: String,
    #[serde(default)]
    pub counterevidence: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityConfidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityAttackPath {
    pub attacker: String,
    pub entrypoint: String,
    #[serde(default)]
    pub preconditions: Vec<String>,
    pub path: Vec<String>,
    pub likelihood: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedSecurityFinding {
    pub finding_id: String,
    pub fingerprint: String,
    pub candidate_id: String,
    pub severity: SecuritySeverity,
    pub source_path: String,
    pub impact: String,
    pub poc_outcome: SecurityPocOutcome,
    pub positive_receipt_id: String,
    pub negative_receipt_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanSeal {
    pub contract_version: u32,
    pub scan_id: String,
    pub model: String,
    pub scope: String,
    pub inventory_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_passes: Option<usize>,
    pub bundle_digest: String,
    pub reviewed_files: usize,
    pub excluded_files: usize,
    pub supporting_files: usize,
    pub candidate_count: usize,
    pub poc_attempted_count: usize,
    pub poc_reproduced_count: usize,
    pub findings: Vec<SealedSecurityFinding>,
}

pub async fn collect_security_inventory(
    exec: &dyn Executor,
    root: &Path,
    scope: &Path,
) -> Result<SecurityInventory, String> {
    if !scope.starts_with(root) {
        return Err(format!(
            "security scope {} is outside project root {}",
            scope.display(),
            root.display()
        ));
    }
    let metadata = exec.metadata(scope).await?;
    if !metadata.is_dir {
        return Err("security scan scope must be a directory".into());
    }
    let scope_name = display_relative(root, scope);
    let mut snapshot = exec
        .walk(scope)
        .await?
        .into_iter()
        .filter_map(|entry| {
            let relative = entry.path.strip_prefix(root).ok()?;
            let path = model_path(relative);
            if is_security_output(&path) {
                return None;
            }
            Some((entry.path.clone(), path))
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.1.cmp(&right.1));
    snapshot.dedup_by(|left, right| left.1 == right.1);
    if snapshot.len() > MAX_INVENTORY_FILES {
        return Err(format!(
            "security inventory exceeds the {MAX_INVENTORY_FILES}-file limit"
        ));
    }
    let mut content_snapshot = Vec::with_capacity(snapshot.len());
    for (absolute, path) in &snapshot {
        let bytes = exec
            .read(absolute)
            .await
            .map_err(|error| format!("cannot read security inventory file `{path}`: {error}"))?;
        content_snapshot.push((path.clone(), hex_digest(&bytes)));
    }
    let inventory_id = inventory_snapshot_digest(&scope_name, &content_snapshot);
    let paths = snapshot
        .into_iter()
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    Ok(SecurityInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: scope_name,
        inventory_id,
        paths,
    })
}

fn display_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        model_path(relative)
    }
}

fn model_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_security_output(path: &str) -> bool {
    path == ".agent/security-scans" || path.starts_with(".agent/security-scans/")
}

#[cfg(test)]
#[path = "security_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "security_deep_tests.rs"]
mod deep_tests;

#[cfg(test)]
#[path = "security_poc_tests.rs"]
mod poc_tests;
