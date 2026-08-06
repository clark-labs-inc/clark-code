//! Native Clark Code artifact transfer boundary.
//!
//! Local paths and S3 upload leases stay out of the WebView's durable state.
//! The host confines source reads to one conversation workspace, hashes exact
//! bytes, uploads without a bearer token, and returns only the authenticated
//! Clark API URI.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

mod workspace_file;

use super::cloud_authority::current_cloud_access;
use super::{clark_http_client, read_json_or_err};
use crate::AppState;

const MAX_DESKTOP_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const WORKSPACE_SCHEME: &str = "clark-workspace://";

#[derive(Debug, Deserialize)]
struct UploadHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct InitiateResponse {
    artifact: Value,
    upload_url: Option<String>,
    #[serde(default)]
    upload_headers: Vec<UploadHeader>,
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
        .ok_or_else(|| "no Clark workspace directory".to_string())
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

fn canonical_cloud_artifact_uri(uri: &str) -> Result<&str, String> {
    if uri.contains(['?', '#']) || !uri.starts_with("/api/desktop/conversations/") {
        return Err("invalid Clark artifact URI".into());
    }
    let parts = uri.split('/').collect::<Vec<_>>();
    if parts.len() != 7
        || parts[1] != "api"
        || parts[2] != "desktop"
        || parts[3] != "conversations"
        || parts[5] != "artifacts"
        || parts[4].is_empty()
        || parts[6].is_empty()
    {
        return Err("invalid Clark artifact URI".into());
    }
    if parts.iter().any(|part| matches!(*part, "." | "..")) {
        return Err("invalid Clark artifact URI".into());
    }
    Ok(uri)
}

#[tauri::command]
pub async fn desktop_artifact_upload(
    desktop_id: String,
    logical_id: String,
    source_uri: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let checked = markdown_source_file(&source_uri, &desktop_id)?;
    let filename = checked.filename.clone();
    let bytes = tokio::task::spawn_blocking(move || {
        workspace_file::read_checked_bytes(checked, MAX_DESKTOP_ARTIFACT_BYTES)
    })
    .await
    .map_err(|error| format!("artifact read task failed: {error}"))??;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    tracing::info!(
        event = "artifact_cloud_initiate",
        conversation_id = %desktop_id,
        size_bytes = bytes.len(),
        "starting Clark cloud artifact publication"
    );
    let initiate_url = format!(
        "{}/api/desktop/conversations/{}/artifacts/initiate",
        access.rest_base,
        urlencoding::encode(&desktop_id)
    );
    let response = clark_http_client()?
        .post(initiate_url)
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "logical_id": logical_id,
            "filename": filename,
            "size_bytes": bytes.len(),
            "sha256": sha256,
        }))
        .send()
        .await
        .map_err(|error| format!("artifact initiation failed: {error}"))?;
    let initiation_status = response.status();
    if !initiation_status.is_success() {
        tracing::warn!(
            event = "artifact_cloud_initiate_failed",
            conversation_id = %desktop_id,
            status = %initiation_status,
            "Clark cloud rejected artifact initiation"
        );
    }
    let initiated: InitiateResponse =
        serde_json::from_value(read_json_or_err(response, "artifact initiation").await?)
            .map_err(|error| format!("artifact initiation returned invalid data: {error}"))?;
    let artifact_id = initiated
        .artifact
        .get("artifact_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "artifact initiation omitted its id".to_string())?;
    tracing::info!(
        event = "artifact_cloud_initiated",
        conversation_id = %desktop_id,
        artifact_id,
        requires_upload = initiated.upload_url.is_some(),
        "Clark cloud artifact initiation accepted"
    );

    if let Some(upload_url) = initiated.upload_url {
        let mut upload = clark_http_client()?.put(upload_url).body(bytes);
        for header in initiated.upload_headers {
            let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| "artifact upload returned an invalid header name".to_string())?;
            let value = reqwest::header::HeaderValue::from_str(&header.value)
                .map_err(|_| "artifact upload returned an invalid header value".to_string())?;
            upload = upload.header(name, value);
        }
        let response = upload
            .send()
            .await
            .map_err(|error| format!("artifact upload failed: {error}"))?;
        if !response.status().is_success() {
            tracing::warn!(
                event = "artifact_cloud_upload_failed",
                conversation_id = %desktop_id,
                artifact_id,
                status = %response.status(),
                "artifact object upload failed"
            );
            return Err(format!("artifact upload failed ({})", response.status()));
        }
        let complete_url = format!(
            "{}/api/desktop/conversations/{}/artifacts/{}/complete",
            access.rest_base,
            urlencoding::encode(&desktop_id),
            urlencoding::encode(artifact_id)
        );
        let response = clark_http_client()?
            .post(complete_url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|error| format!("artifact completion failed: {error}"))?;
        tracing::info!(
            event = "artifact_cloud_completion_received",
            conversation_id = %desktop_id,
            artifact_id,
            status = %response.status(),
            "Clark cloud artifact completion returned"
        );
        return read_json_or_err(response, "artifact completion").await;
    }
    Ok(initiated.artifact)
}

#[tauri::command]
pub async fn desktop_artifact_read(
    uri: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let uri = canonical_cloud_artifact_uri(&uri)?;
    let account = state
        .runtime_registry
        .cloud_account()
        .await
        .ok_or_else(|| "Clark has no active signed-in account".to_string())?;
    let response = clark_http_client()?
        .get(format!("{}{}", account.rest_base, uri))
        .bearer_auth(account.token.as_str())
        .send()
        .await
        .map_err(|error| format!("artifact download failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("artifact download failed ({})", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_DESKTOP_ARTIFACT_BYTES)
    {
        return Err("cloud artifact exceeds the 8 MB limit".into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.starts_with("text/markdown") {
        return Err("cloud artifact is not Markdown".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("artifact download failed: {error}"))?;
    if bytes.len() as u64 > MAX_DESKTOP_ARTIFACT_BYTES {
        return Err("cloud artifact exceeds the 8 MB limit".into());
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| "cloud artifact is not valid UTF-8 Markdown".to_string())
}

#[tauri::command]
pub async fn desktop_artifact_read_workspace(uri: String) -> Result<String, String> {
    let desktop_id = workspace_uri_session(&uri)?;
    let checked = markdown_source_file(&uri, &desktop_id)?;
    let bytes = tokio::task::spawn_blocking(move || {
        workspace_file::read_checked_bytes(checked, MAX_DESKTOP_ARTIFACT_BYTES)
    })
    .await
    .map_err(|error| format!("artifact read task failed: {error}"))??;
    String::from_utf8(bytes)
        .map_err(|_| "workspace artifact is not valid UTF-8 Markdown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_uri_accepts_only_the_authenticated_artifact_route() {
        assert!(canonical_cloud_artifact_uri(
            "/api/desktop/conversations/desk-1/artifacts/deskart_1"
        )
        .is_ok());
        assert!(canonical_cloud_artifact_uri("https://evil.test/file.md").is_err());
        assert!(
            canonical_cloud_artifact_uri("/api/desktop/conversations/../artifacts/deskart_1")
                .is_err()
        );
        assert!(canonical_cloud_artifact_uri(
            "/api/desktop/conversations/desk-1/artifacts/deskart_1?share=1"
        )
        .is_err());
        assert!(canonical_cloud_artifact_uri(
            "/api/desktop/conversations/desk-1/artifacts/deskart_1/"
        )
        .is_err());
    }

    #[test]
    fn pending_workspace_uri_is_conversation_bound_and_traversal_safe() {
        assert_eq!(
            relative_workspace_source("clark-workspace://desk-1/report.md", "desk-1")
                .unwrap()
                .unwrap(),
            PathBuf::from("report.md")
        );
        assert!(relative_workspace_source("clark-workspace://other/report.md", "desk-1").is_err());
        assert!(
            relative_workspace_source("clark-workspace://desk-1/../secret.md", "desk-1").is_err()
        );
        assert_eq!(
            workspace_uri_session("clark-workspace://desk-1/report.md").unwrap(),
            "desk-1"
        );
        assert!(workspace_uri_session("clark-workspace://%2E%2E/report.md").is_err());
        assert!(workspace_uri_session("clark-workspace://%2Fetc/report.md").is_err());
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
