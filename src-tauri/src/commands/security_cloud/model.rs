use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub(super) struct ArtifactSpec {
    pub role: &'static str,
    pub storage_tier: &'static str,
    pub classification: &'static str,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScannerEnrollment {
    pub id: String,
    pub organization_id: String,
    pub signer_id: String,
    pub public_key: String,
    pub kind: String,
    pub display_name: String,
    pub enrolled_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlatformScan {
    pub id: String,
    pub organization_id: String,
    pub repository_id: String,
    pub client_scan_id: String,
    pub snapshot_digest: String,
    pub inventory_id: String,
    pub model: String,
    pub status: String,
    pub version: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlatformTask {
    pub id: String,
    pub organization_id: String,
    pub repository_id: String,
    pub scan_id: String,
    pub task_kind: String,
    pub lease_fence: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PlatformTaskMutation {
    pub task: PlatformTask,
    pub scan: PlatformScan,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UploadHeader {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArtifactAuthorization {
    pub id: String,
    pub organization_id: String,
    pub repository_id: String,
    pub scan_id: String,
    pub client_artifact_id: String,
    pub role: String,
    pub storage_tier: String,
    pub classification: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub status: String,
    pub object_version_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArtifactUploadGrant {
    pub authorization: ArtifactAuthorization,
    pub upload_url: Option<String>,
    pub upload_headers: Vec<UploadHeader>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArtifactRecord {
    pub id: String,
    pub scan_id: String,
    pub client_artifact_id: String,
    pub role: String,
    pub storage_tier: String,
    pub classification: String,
    pub object_version_id: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlatformSealReceipt {
    pub receipt_key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PlatformSealResult {
    pub scan: PlatformScan,
    pub receipt: PlatformSealReceipt,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalCloudReceipt {
    pub product: String,
    pub schema_version: u32,
    pub organization_id: String,
    pub repository_id: String,
    pub local_scan_id: String,
    pub local_bundle_digest: String,
    pub platform_scan_id: String,
    pub platform_seal_receipt_key: Option<String>,
    pub synced_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCloudScanSyncStatus {
    Synced,
    AlreadySynced,
    Pending,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCloudScanSync {
    pub local_scan_id: String,
    pub platform_scan_id: Option<String>,
    pub status: SecurityCloudScanSyncStatus,
    pub seal_receipt_key: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCloudSyncResult {
    pub sealed_scan_count: usize,
    pub synced_count: usize,
    pub already_synced_count: usize,
    pub pending_count: usize,
    pub failed_count: usize,
    pub scans: Vec<SecurityCloudScanSync>,
}

impl SecurityCloudSyncResult {
    pub(super) fn from_scans(sealed_scan_count: usize, scans: Vec<SecurityCloudScanSync>) -> Self {
        let synced_count = scans
            .iter()
            .filter(|scan| matches!(scan.status, SecurityCloudScanSyncStatus::Synced))
            .count();
        let already_synced_count = scans
            .iter()
            .filter(|scan| matches!(scan.status, SecurityCloudScanSyncStatus::AlreadySynced))
            .count();
        let pending_count = scans
            .iter()
            .filter(|scan| matches!(scan.status, SecurityCloudScanSyncStatus::Pending))
            .count();
        let failed_count = scans
            .iter()
            .filter(|scan| matches!(scan.status, SecurityCloudScanSyncStatus::Failed))
            .count();
        Self {
            sealed_scan_count,
            synced_count,
            already_synced_count,
            pending_count,
            failed_count,
            scans,
        }
    }
}
