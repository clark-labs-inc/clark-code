use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::runtime_registry::SessionKey;
use crate::AppState;

const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

/// Read a JSON artifact only from the live conversation's native-owned local roots.
#[tauri::command]
pub async fn read_artifact_text(
    path: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let key = SessionKey::parse(session_id)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&key)
        .await
        .ok_or("no such conversation")?;
    let session = entry.lock().await;
    let environment = session
        .session
        .environment
        .as_ref()
        .ok_or("this conversation has no filesystem binding")?;
    if environment.remote {
        return Err("remote artifacts cannot be read from this device".into());
    }
    let roots = environment
        .workspace_roots
        .iter()
        .chain(environment.checkout_root.iter())
        .chain(environment.docs_root.iter())
        .filter_map(|root| PathBuf::from(root).canonicalize().ok())
        .collect::<Vec<_>>();
    drop(session);
    tokio::task::spawn_blocking(move || read_json_in_roots(&path, &roots))
        .await
        .map_err(|error| format!("artifact read failed: {error}"))?
}

fn read_json_in_roots(path: &str, roots: &[PathBuf]) -> Result<String, String> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|error| format!("artifact path: {error}"))?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("artifact is outside this conversation's workspace".into());
    }
    if !canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err("only JSON artifacts can be previewed as text".into());
    }
    let metadata = std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_PREVIEW_BYTES {
        return Err("artifact is unavailable or too large to preview".into());
    }
    let file = std::fs::File::open(&canonical)
        .map_err(|error| format!("artifact read failed: {error}"))?;
    let mut bounded = file.take(MAX_PREVIEW_BYTES + 1);
    let mut text = String::new();
    bounded
        .read_to_string(&mut text)
        .map_err(|error| format!("artifact read failed: {error}"))?;
    if text.len() as u64 > MAX_PREVIEW_BYTES {
        return Err("artifact is too large to preview".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::read_json_in_roots;

    #[test]
    fn json_preview_is_limited_to_the_session_root_and_size() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let roots = vec![root.path().canonicalize().unwrap()];
        let allowed = root.path().join("research.json");
        let forbidden = outside.path().join("research.json");
        let wrong_type = root.path().join("notes.txt");
        std::fs::write(&allowed, r#"{"result":"ready"}"#).unwrap();
        std::fs::write(&forbidden, "outside").unwrap();
        std::fs::write(&wrong_type, "text").unwrap();
        assert_eq!(
            read_json_in_roots(allowed.to_str().unwrap(), &roots).unwrap(),
            r#"{"result":"ready"}"#
        );
        assert!(read_json_in_roots(forbidden.to_str().unwrap(), &roots).is_err());
        assert!(read_json_in_roots(wrong_type.to_str().unwrap(), &roots).is_err());
        std::fs::write(&allowed, vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(read_json_in_roots(allowed.to_str().unwrap(), &roots).is_err());
    }
}
