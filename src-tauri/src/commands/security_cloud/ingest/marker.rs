use std::path::{Component, Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;

use super::super::model::LocalCloudReceipt;

const MAX_MARKER_BYTES: u64 = 64 * 1024;

pub(super) async fn marker_path(
    root: &Path,
    record: &provider_local::SecurityScanRecord,
    organization_id: &str,
    repository_id: &str,
) -> Result<PathBuf, String> {
    let relative = Path::new(&record.path);
    if relative.is_absolute()
        || relative.file_name().is_none_or(|name| name != "scan.json")
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Clark Security history returned an unsafe scan path".into());
    }
    let scan_dir = root
        .join(relative)
        .parent()
        .ok_or_else(|| "Clark Security scan path has no parent".to_string())?
        .to_path_buf();
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| format!("cannot resolve Clark repository root: {error}"))?;
    let canonical_scan_dir = tokio::fs::canonicalize(&scan_dir)
        .await
        .map_err(|error| format!("cannot resolve Clark Security scan directory: {error}"))?;
    if !canonical_scan_dir.starts_with(&canonical_root) {
        return Err("Clark Security scan directory resolves outside the repository".into());
    }
    let binding = sha256_hex(format!("{organization_id}\0{repository_id}").as_bytes());
    Ok(canonical_scan_dir
        .join("cloud")
        .join(format!("{binding}.json")))
}

pub(super) async fn read_matching_marker(
    path: &Path,
    organization_id: &str,
    repository_id: &str,
    local_scan_id: &str,
    local_bundle_digest: &str,
) -> Result<Option<LocalCloudReceipt>, String> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot inspect Clark Security cloud receipt: {error}"
            ))
        }
    };
    if !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES {
        return Ok(None);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| format!("cannot read Clark Security cloud receipt: {error}"))?;
    let Ok(marker) = serde_json::from_slice::<LocalCloudReceipt>(&bytes) else {
        return Ok(None);
    };
    Ok((marker.product == "Clark Security"
        && marker.schema_version == 1
        && marker.organization_id == organization_id
        && marker.repository_id == repository_id
        && marker.local_scan_id == local_scan_id
        && marker.local_bundle_digest == local_bundle_digest
        && uuid::Uuid::parse_str(&marker.platform_scan_id).is_ok())
    .then_some(marker))
}

pub(super) async fn write_marker(path: &Path, marker: &LocalCloudReceipt) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Clark Security cloud receipt has no parent".to_string())?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        format!("cannot create Clark Security cloud receipt directory: {error}")
    })?;
    let mut bytes = serde_json::to_vec_pretty(marker)
        .map_err(|error| format!("cannot encode Clark Security cloud receipt: {error}"))?;
    bytes.push(b'\n');
    let temporary = parent.join(format!(".sync-{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|error| format!("cannot create Clark Security cloud receipt: {error}"))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("cannot write Clark Security cloud receipt: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("cannot sync Clark Security cloud receipt: {error}"))?;
    drop(file);
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| format!("cannot publish Clark Security cloud receipt: {error}"))
}

pub(super) fn now_ms() -> Result<i64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| "system clock exceeds the Clark Security timestamp range".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
