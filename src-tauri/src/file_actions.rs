use std::path::PathBuf;

use base64::Engine as _;

const MAX_EMBEDDED_ARTIFACT_BYTES: usize = 25 * 1024 * 1024;

#[tauri::command]
pub async fn copy_local_file(source: String, destination: String) -> Result<(), String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    tauri::async_runtime::spawn_blocking(move || copy_file(&source, &destination))
        .await
        .map_err(|error| format!("copy task failed: {error}"))?
}

/// Save either an existing local file or a bounded base64 `data:` artifact to
/// the user-selected destination. This keeps binary artifact downloads native
/// without granting the WebView broad filesystem write access.
#[tauri::command]
pub async fn save_artifact_copy(source: String, destination: String) -> Result<(), String> {
    let destination = PathBuf::from(destination);
    tauri::async_runtime::spawn_blocking(move || save_artifact(&source, &destination))
        .await
        .map_err(|error| format!("save task failed: {error}"))?
}

fn save_artifact(source: &str, destination: &std::path::Path) -> Result<(), String> {
    if source.starts_with("data:") {
        return save_data_url(source, destination);
    }
    copy_file(std::path::Path::new(source), destination)
}

fn save_data_url(source: &str, destination: &std::path::Path) -> Result<(), String> {
    let (metadata, payload) = source
        .split_once(',')
        .ok_or_else(|| "embedded artifact is malformed".to_string())?;
    if !metadata.ends_with(";base64") {
        return Err("embedded artifact must use base64 encoding".into());
    }
    if payload.len() > MAX_EMBEDDED_ARTIFACT_BYTES.saturating_mul(4).div_ceil(3) + 4 {
        return Err("embedded artifact is too large to save".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("embedded artifact is invalid base64: {error}"))?;
    if bytes.len() > MAX_EMBEDDED_ARTIFACT_BYTES {
        return Err("embedded artifact is too large to save".into());
    }
    std::fs::write(destination, bytes)
        .map_err(|error| format!("couldn't save {}: {error}", destination.display()))
}

fn copy_file(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    let metadata = std::fs::metadata(source)
        .map_err(|error| format!("couldn't read {}: {error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("couldn't save {}: {error}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::{copy_file, save_artifact};

    #[test]
    fn copies_a_local_file_to_the_selected_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.docx");
        let destination = dir.path().join("saved.docx");
        std::fs::write(&source, b"document bytes").unwrap();

        copy_file(&source, &destination).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"document bytes");
    }

    #[test]
    fn rejects_a_directory_source() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("copy");

        let error = copy_file(dir.path(), &destination).unwrap_err();

        assert!(error.contains("is not a file"));
    }

    #[test]
    fn saves_a_base64_data_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("preview.png");

        save_artifact("data:image/png;base64,AQIDBA==", &destination).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn rejects_non_base64_data_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("preview.txt");

        let error = save_artifact("data:text/plain,hello", &destination).unwrap_err();

        assert!(error.contains("base64"));
        assert!(!destination.exists());
    }
}
