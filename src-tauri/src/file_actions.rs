use std::path::PathBuf;

#[tauri::command]
pub async fn copy_local_file(source: String, destination: String) -> Result<(), String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    tauri::async_runtime::spawn_blocking(move || copy_file(&source, &destination))
        .await
        .map_err(|error| format!("copy task failed: {error}"))?
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
    use super::copy_file;

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
}
