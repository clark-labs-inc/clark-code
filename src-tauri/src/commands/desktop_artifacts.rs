//! Product-neutral, handle-bound reads from conversation workspaces.

use std::path::{Component, Path, PathBuf};

mod workspace_file;

const MAX_DESKTOP_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const WORKSPACE_SCHEME: &str = "workspace-artifact://";

fn validate_workspace_session(session: &str) -> Result<(), String> {
    let path = Path::new(session);
    let mut components = path.components();
    if session.is_empty()
        || session.contains(['/', '\\'])
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("invalid workspace artifact session".into());
    }
    Ok(())
}

fn session_workspace_path(desktop_id: &str) -> Result<PathBuf, String> {
    validate_workspace_session(desktop_id)?;
    provider_local::session_workspace(desktop_id)
        .ok_or_else(|| "no Agent Desktop workspace directory".to_string())
}

fn relative_workspace_source(source: &str, desktop_id: &str) -> Result<Option<PathBuf>, String> {
    let Some(value) = source.strip_prefix(WORKSPACE_SCHEME) else {
        return Ok(None);
    };
    let (encoded_session, encoded_relative) = value
        .split_once('/')
        .ok_or_else(|| "invalid workspace artifact URI".to_string())?;
    let session = urlencoding::decode(encoded_session)
        .map_err(|_| "invalid workspace artifact session".to_string())?;
    validate_workspace_session(session.as_ref())?;
    if session.as_ref() != desktop_id {
        return Err("workspace artifact belongs to another conversation".into());
    }
    let relative = urlencoding::decode(encoded_relative)
        .map_err(|_| "invalid workspace artifact path".to_string())?;
    let path = Path::new(relative.as_ref());
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("workspace artifact path is not confined".into());
    }
    Ok(Some(path.to_path_buf()))
}

fn workspace_uri_session(source: &str) -> Result<String, String> {
    let value = source
        .strip_prefix(WORKSPACE_SCHEME)
        .ok_or_else(|| "invalid workspace artifact URI".to_string())?;
    let encoded_session = value
        .split_once('/')
        .map(|(session, _)| session)
        .ok_or_else(|| "invalid workspace artifact URI".to_string())?;
    let session = urlencoding::decode(encoded_session)
        .map(|value| value.into_owned())
        .map_err(|_| "invalid workspace artifact session".to_string())?;
    validate_workspace_session(&session)?;
    Ok(session)
}

fn workspace_relative_source(
    source: &str,
    desktop_id: &str,
    workspace: &Path,
) -> Result<PathBuf, String> {
    if let Some(relative) = relative_workspace_source(source, desktop_id)? {
        return Ok(relative);
    }
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace is unavailable: {error}"))?;
    let canonical = Path::new(source)
        .canonicalize()
        .map_err(|error| format!("artifact is unavailable: {error}"))?;
    let relative = canonical
        .strip_prefix(&canonical_workspace)
        .map_err(|_| "artifact is not Markdown in this conversation workspace".to_string())?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("artifact is not Markdown in this conversation workspace".into());
    }
    Ok(relative.to_path_buf())
}

fn markdown_source_file(
    source: &str,
    desktop_id: &str,
) -> Result<workspace_file::CheckedMarkdown, String> {
    let workspace = session_workspace_path(desktop_id)?;
    let relative = workspace_relative_source(source, desktop_id, &workspace)?;
    workspace_file::open_markdown_file(&workspace, &relative, MAX_DESKTOP_ARTIFACT_BYTES)
}

pub(crate) async fn read_workspace_markdown(
    source_uri: &str,
    conversation_id: &str,
) -> Result<(String, Vec<u8>), String> {
    let checked = markdown_source_file(source_uri, conversation_id)?;
    let filename = checked.filename.clone();
    let bytes = tokio::task::spawn_blocking(move || {
        workspace_file::read_checked_bytes(checked, MAX_DESKTOP_ARTIFACT_BYTES)
    })
    .await
    .map_err(|error| format!("artifact read task failed: {error}"))??;
    Ok((filename, bytes))
}

#[tauri::command]
pub async fn workspace_artifact_read(uri: String) -> Result<String, String> {
    let desktop_id = workspace_uri_session(&uri)?;
    let (_, bytes) = read_workspace_markdown(&uri, &desktop_id).await?;
    String::from_utf8(bytes)
        .map_err(|_| "workspace artifact is not valid UTF-8 Markdown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_workspace_uri_is_conversation_bound_and_traversal_safe() {
        assert_eq!(
            relative_workspace_source("workspace-artifact://desk-1/report.md", "desk-1")
                .unwrap()
                .unwrap(),
            PathBuf::from("report.md")
        );
        assert!(
            relative_workspace_source("workspace-artifact://other/report.md", "desk-1").is_err()
        );
        assert!(
            relative_workspace_source("workspace-artifact://desk-1/../secret.md", "desk-1")
                .is_err()
        );
        assert_eq!(
            workspace_uri_session("workspace-artifact://desk-1/report.md").unwrap(),
            "desk-1"
        );
        assert!(workspace_uri_session("workspace-artifact://%2E%2E/report.md").is_err());
        assert!(workspace_uri_session("workspace-artifact://%2Fetc/report.md").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_read_keeps_the_checked_file_handle_after_a_path_swap() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace directory");
        let artifact = workspace.join("report.md");
        std::fs::write(&artifact, b"# checked artifact\n").expect("write artifact");
        let checked = workspace_file::open_markdown_file(
            &workspace,
            Path::new("report.md"),
            MAX_DESKTOP_ARTIFACT_BYTES,
        )
        .expect("open checked artifact");

        let outside = root.path().join("outside.md");
        std::fs::write(&outside, b"# outside bytes\n").expect("write outside file");
        std::fs::rename(&artifact, workspace.join("checked.md")).expect("rename artifact");
        symlink(&outside, &artifact).expect("replace artifact path with symlink");

        let bytes = workspace_file::read_checked_bytes(checked, MAX_DESKTOP_ARTIFACT_BYTES)
            .expect("read checked handle");
        assert_eq!(bytes, b"# checked artifact\n");
    }

    #[cfg(unix)]
    #[test]
    fn workspace_read_rejects_a_symlinked_artifact() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace directory");
        let outside = root.path().join("outside.md");
        std::fs::write(&outside, b"# outside bytes\n").expect("write outside file");
        symlink(&outside, workspace.join("report.md")).expect("create artifact symlink");

        assert!(workspace_file::open_markdown_file(
            &workspace,
            Path::new("report.md"),
            MAX_DESKTOP_ARTIFACT_BYTES,
        )
        .is_err());
    }
}
