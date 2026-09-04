//! Product-neutral, handle-bound reads from conversation workspaces.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};

use super::cloud_authority::current_account_access;
use crate::runtime_registry::SessionKey;
use crate::AppState;

mod workspace_file;

const MAX_DESKTOP_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const WORKSPACE_SCHEME: &str = "workspace-artifact://";
const STAGED_ARTIFACT_DIR: &str = ".clark-sync";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedArtifactReceipt {
    source_uri: String,
    sha256: String,
    remote_uri: Option<String>,
}

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
        .ok_or_else(|| "no Clark Code workspace directory".to_string())
}

fn workspace_path(desktop_id: &str, workspace: Option<&Path>) -> Result<PathBuf, String> {
    workspace
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| session_workspace_path(desktop_id))
}

async fn live_workspace_path(desktop_id: &str, state: &AppState) -> Result<PathBuf, String> {
    let session_key = SessionKey::parse(desktop_id.to_string())?;
    if let Some(entry) = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
    {
        let session = entry.lock().await;
        if let Some(root) = session
            .session
            .environment
            .as_ref()
            .and_then(|environment| environment.docs_root.as_deref())
        {
            return Ok(PathBuf::from(root));
        }
    }
    session_workspace_path(desktop_id)
}

fn staged_source_uri(desktop_id: &str, filename: &str) -> String {
    format!(
        "{WORKSPACE_SCHEME}{}/{STAGED_ARTIFACT_DIR}/{}",
        urlencoding::encode(desktop_id),
        urlencoding::encode(filename),
    )
}

fn materialize_staged_artifact(
    workspace: &Path,
    filename: &str,
    markdown: &[u8],
) -> Result<(), String> {
    let directory = workspace.join(STAGED_ARTIFACT_DIR);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not prepare durable artifact staging: {error}"))?;
    let destination = directory.join(filename);
    let temporary = directory.join(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("could not create durable artifact stage: {error}"))?;
    if let Err(error) = file.write_all(markdown).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("could not commit durable artifact stage: {error}"));
    }
    drop(file);
    std::fs::rename(&temporary, &destination)
        .map_err(|error| format!("could not publish durable artifact stage: {error}"))?;
    if let Ok(directory) = std::fs::File::open(&directory) {
        let _ = directory.sync_all();
    }
    Ok(())
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
    workspace: Option<&Path>,
) -> Result<workspace_file::CheckedMarkdown, String> {
    let workspace = workspace_path(desktop_id, workspace)?;
    let relative = workspace_relative_source(source, desktop_id, &workspace)?;
    workspace_file::open_markdown_file(&workspace, &relative, MAX_DESKTOP_ARTIFACT_BYTES)
}

pub(crate) async fn read_workspace_markdown(
    source_uri: &str,
    conversation_id: &str,
) -> Result<(String, Vec<u8>), String> {
    read_workspace_markdown_in(source_uri, conversation_id, None).await
}

pub(crate) async fn read_workspace_markdown_in(
    source_uri: &str,
    conversation_id: &str,
    workspace: Option<PathBuf>,
) -> Result<(String, Vec<u8>), String> {
    let checked = markdown_source_file(source_uri, conversation_id, workspace.as_deref())?;
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

/// Copy generated Markdown into both the FULL-synchronous native journal and a
/// conversation-confined workspace file before cloud upload begins. The bytes
/// and stable logical id survive an updater-forced process exit; a later
/// renderer can rematerialize the file and continue the same upload.
#[tauri::command]
pub async fn desktop_artifact_stage(
    app: AppHandle,
    desktop_id: String,
    logical_id: String,
    source_uri: String,
    state: State<'_, AppState>,
) -> Result<StagedArtifactReceipt, String> {
    if logical_id.trim().is_empty() || logical_id.len() > 1024 {
        return Err("artifact logical id is invalid".into());
    }
    let access = current_account_access(state.inner()).await?;
    let outbox_path = crate::trajectory::outbox_path(&app)?;
    let workspace = live_workspace_path(&desktop_id, state.inner()).await?;
    let existing = crate::trajectory::staged_artifact(
        outbox_path.clone(),
        access.owner_scope.clone(),
        desktop_id.clone(),
        logical_id.clone(),
    )
    .await?;

    let staged =
        match read_workspace_markdown_in(&source_uri, &desktop_id, Some(workspace.clone())).await {
            Ok((filename, markdown)) => {
                let sha256 = hex::encode(Sha256::digest(&markdown));
                crate::trajectory::stage_artifact(
                    outbox_path,
                    access.owner_scope,
                    desktop_id.clone(),
                    logical_id,
                    filename,
                    markdown,
                    sha256,
                )
                .await?
            }
            Err(error) => existing.ok_or(error)?,
        };
    let digest_prefix = staged
        .sha256
        .get(..16)
        .filter(|prefix| prefix.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("durable artifact stage has an invalid digest")?;
    let filename = format!("{digest_prefix}-{}", staged.filename);
    let markdown = staged.markdown.clone();
    let workspace_for_write = workspace.clone();
    let filename_for_write = filename.clone();
    tokio::task::spawn_blocking(move || {
        materialize_staged_artifact(&workspace_for_write, &filename_for_write, &markdown)
    })
    .await
    .map_err(|error| format!("durable artifact stage task failed: {error}"))??;

    Ok(StagedArtifactReceipt {
        source_uri: staged_source_uri(&desktop_id, &filename),
        sha256: staged.sha256,
        remote_uri: staged.remote_uri,
    })
}

#[tauri::command]
pub async fn desktop_artifact_mark_uploaded(
    app: AppHandle,
    desktop_id: String,
    logical_id: String,
    sha256: String,
    remote_uri: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !remote_uri.starts_with("/api/desktop/conversations/") {
        return Err("uploaded artifact URI is invalid".into());
    }
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("uploaded artifact digest is invalid".into());
    }
    let access = current_account_access(state.inner()).await?;
    crate::trajectory::mark_staged_artifact_uploaded(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        desktop_id,
        logical_id,
        sha256,
        remote_uri,
    )
    .await
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

    #[tokio::test]
    async fn artifact_reads_use_the_live_session_document_root() {
        let root = tempfile::tempdir().expect("temporary directory");
        let artifact = root.path().join("report.md");
        std::fs::write(&artifact, b"# live artifact\n").expect("write artifact");

        let result = read_workspace_markdown_in(
            artifact.to_string_lossy().as_ref(),
            "conversation-1",
            Some(root.path().to_path_buf()),
        )
        .await
        .expect("read artifact from live docs root");

        assert_eq!(
            result,
            ("report.md".to_string(), b"# live artifact\n".to_vec())
        );
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
