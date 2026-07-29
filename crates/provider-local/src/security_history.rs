use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use crate::exec::Executor;
use crate::security::{SecurityPocReceipt, SecurityScanBundle, SecurityScanSeal};

const MAX_HISTORY_SCANS: usize = 200;
const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanRecord {
    pub path: String,
    pub modified_at_ms: Option<u64>,
    pub bundle: SecurityScanBundle,
    pub seal: Option<SecurityScanSeal>,
    pub poc_receipts: Vec<SecurityPocReceipt>,
}

pub async fn list_security_scans(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Vec<SecurityScanRecord>, String> {
    let scans_root = root.join(".clark/security-scans");
    let metadata = match exec.metadata(&scans_root).await {
        Ok(metadata) => metadata,
        Err(_) => return Ok(Vec::new()),
    };
    if !metadata.is_dir {
        return Ok(Vec::new());
    }
    let entries = exec.walk(&scans_root).await?;
    let mut artifacts = entries
        .iter()
        .filter(|entry| {
            entry
                .path
                .file_name()
                .is_some_and(|name| name == "scan.json")
        })
        .cloned()
        .collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.path.cmp(&left.path))
    });
    artifacts.truncate(MAX_HISTORY_SCANS);

    let mut records = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        if artifact.len as usize > MAX_ARTIFACT_BYTES {
            continue;
        }
        let bytes = match exec.read(&artifact.path).await {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let bundle = match serde_json::from_slice::<SecurityScanBundle>(&bytes) {
            Ok(bundle) => bundle,
            Err(_) => continue,
        };
        let seal_path = artifact.path.with_file_name("seal.json");
        let seal = match exec.read(&seal_path).await {
            Ok(bytes) if bytes.len() <= MAX_ARTIFACT_BYTES => {
                serde_json::from_slice::<SecurityScanSeal>(&bytes).ok()
            }
            _ => None,
        };
        let poc_receipts = load_poc_receipts(exec, &artifact.path, &bundle, &entries).await;
        let path = artifact
            .path
            .strip_prefix(root)
            .unwrap_or(&artifact.path)
            .to_string_lossy()
            .replace('\\', "/");
        let modified_at_ms = artifact
            .modified
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| duration.as_millis().try_into().ok());
        records.push(SecurityScanRecord {
            path,
            modified_at_ms,
            bundle,
            seal,
            poc_receipts,
        });
    }
    Ok(records)
}

async fn load_poc_receipts(
    exec: &dyn Executor,
    scan_artifact: &Path,
    bundle: &SecurityScanBundle,
    entries: &[exec_core::WalkEntry],
) -> Vec<SecurityPocReceipt> {
    let expected = bundle
        .candidates
        .iter()
        .flat_map(|candidate| {
            [
                candidate.poc.positive_receipt_id.as_deref(),
                candidate.poc.negative_receipt_id.as_deref(),
            ]
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    let Some(scan_dir) = scan_artifact.parent() else {
        return Vec::new();
    };
    let mut receipts = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for entry in entries.iter().filter(|entry| {
        entry.path.starts_with(scan_dir)
            && entry
                .path
                .file_name()
                .is_some_and(|name| name == "receipt.json")
            && entry.len as usize <= MAX_ARTIFACT_BYTES
    }) {
        let Ok(bytes) = exec.read(&entry.path).await else {
            continue;
        };
        let Ok(receipt) = serde_json::from_slice::<SecurityPocReceipt>(&bytes) else {
            continue;
        };
        if !expected.contains(receipt.receipt_id.as_str())
            || receipt.scan_id != bundle.scan_id
            || receipt.inventory_id != bundle.inventory_id
        {
            continue;
        }
        let receipt_id = receipt.receipt_id.clone();
        if receipts.insert(receipt_id.clone(), receipt).is_some() {
            ambiguous.insert(receipt_id);
        }
    }
    for receipt_id in ambiguous {
        receipts.remove(&receipt_id);
    }
    receipts.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;
    use crate::security::{
        SecurityCoverage, SecurityCoverageStatus, SecurityScanMode, SecurityScanPhase,
        SecurityThreatModel, SECURITY_SCAN_CONTRACT_VERSION,
    };

    #[tokio::test]
    async fn history_reads_valid_artifacts_and_ignores_malformed_rows() {
        let temp = tempfile::tempdir().unwrap();
        let scans = temp.path().join(".clark/security-scans");
        std::fs::create_dir_all(scans.join("scan-1")).unwrap();
        std::fs::create_dir_all(scans.join("broken")).unwrap();
        let bundle = SecurityScanBundle {
            contract_version: SECURITY_SCAN_CONTRACT_VERSION,
            scan_id: "scan-1".into(),
            mode: SecurityScanMode::Standard,
            model: crate::config::SECURITY_MODEL.into(),
            scope: ".".into(),
            inventory_id: "inventory".into(),
            phase: SecurityScanPhase::Reporting,
            threat_model: SecurityThreatModel {
                assets: vec!["asset".into()],
                trust_boundaries: vec!["boundary".into()],
                attacker_inputs: vec!["input".into()],
                invariants: vec!["invariant".into()],
            },
            coverage: vec![SecurityCoverage {
                path: "src/lib.rs".into(),
                status: SecurityCoverageStatus::Reviewed,
                reason: None,
            }],
            supporting_coverage: Vec::new(),
            diff_target: None,
            deep_run_id: None,
            candidates: Vec::new(),
        };
        std::fs::write(
            scans.join("scan-1/scan.json"),
            serde_json::to_vec(&bundle).unwrap(),
        )
        .unwrap();
        std::fs::write(scans.join("broken/scan.json"), b"not json").unwrap();

        let records = list_security_scans(&LocalExecutor, temp.path())
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, ".clark/security-scans/scan-1/scan.json");
        assert_eq!(records[0].bundle.scan_id, "scan-1");
        assert!(records[0].seal.is_none());
    }
}
