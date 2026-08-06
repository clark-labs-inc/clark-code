//! Clark Code mobile remote-control IPC bridge.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use tauri::State;

use crate::commands::{clark_http_client, current_cloud_access, read_json_or_err, CloudAccess};
use crate::state::AppState;

const HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const ATTACHMENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_POLL_TIMEOUT_SLACK_MS: u64 = 10_000;
const MAX_CODE_REMOTE_ATTACHMENT_BYTES: usize = 12 * 1024 * 1024;

fn trace_command_receipt(command: &Value, boundary: &str) {
    let response = command.get("response");
    let timing = command.get("timing");
    tracing::info!(
        boundary,
        command_id = command
            .get("command_id")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        host_id = command
            .get("host_id")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        instance_id = command
            .get("claim_instance_id")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        desktop_id = response
            .and_then(|value| value.get("desktop_id"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                command
                    .get("desktop_id")
                    .and_then(|value| value.as_str())
            })
            .unwrap_or(""),
        run_id = response
            .and_then(|value| value.get("run_id"))
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        command_type = command
            .get("command_type")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        command_status = command
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        delivery_ms = ?timing
            .and_then(|value| value.get("delivery_ms"))
            .and_then(|value| value.as_i64()),
        acceptance_ms = ?timing
            .and_then(|value| value.get("acceptance_ms"))
            .and_then(|value| value.as_i64()),
        execution_receipt_ms = ?timing
            .and_then(|value| value.get("execution_receipt_ms"))
            .and_then(|value| value.as_i64()),
        total_receipt_ms = ?timing
            .and_then(|value| value.get("total_receipt_ms"))
            .and_then(|value| value.as_i64()),
        failure_code = response
            .and_then(|value| {
                value
                    .get("error_code")
                    .or_else(|| value.get("failure_code"))
            })
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        "Clark Code remote command receipt"
    );
}

#[derive(Debug, Deserialize)]
struct CodeRemoteAttachmentDownloadLease {
    url: String,
    filename: String,
    content_type: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct DownloadedCodeRemoteAttachment {
    filename: String,
    content_type: String,
    size_bytes: usize,
    data_base64: String,
}

/// Register or heartbeat this Clark Code host and publish the projects mobile
/// is allowed to start. Projects are server-validated before any command can
/// target them.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn desktop_code_host_upsert(
    host_id: String,
    display_name: String,
    os_name: String,
    arch: String,
    app_version: String,
    protocol_version: i64,
    capabilities: Value,
    projects: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    code_host_upsert(
        access,
        host_id,
        display_name,
        os_name,
        arch,
        app_version,
        protocol_version,
        capabilities,
        projects,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn code_host_upsert(
    access: CloudAccess,
    host_id: String,
    display_name: String,
    os_name: String,
    arch: String,
    app_version: String,
    protocol_version: i64,
    capabilities: Value,
    projects: Value,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/code/hosts/{}",
        access.rest_base,
        urlencoding::encode(&host_id)
    );
    let resp = clark_http_client()?
        .put(url)
        .timeout(HOST_REQUEST_TIMEOUT)
        .bearer_auth(access.token)
        .json(&serde_json::json!({
            "display_name": display_name,
            "os": os_name,
            "arch": arch,
            "app_version": app_version,
            "protocol_version": protocol_version,
            "capabilities": capabilities,
            "projects": projects,
        }))
        .send()
        .await
        .map_err(|e| format!("Clark Code host upsert request failed: {e}"))?;
    read_json_or_err(resp, "Clark Code host upsert").await
}

/// Poll mobile-originated Clark Code commands for this host. The backend keeps
/// commands durable and may redeliver unacked commands after reconnects.
#[tauri::command]
pub async fn desktop_code_command_poll(
    host_id: String,
    instance_id: String,
    limit: Option<i64>,
    wait_ms: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let account = crate::runtime_registry::AccountKey::new(access.owner_scope.clone())?;
    let mut url = format!(
        "{}/api/desktop/code/hosts/{}/commands",
        access.rest_base,
        urlencoding::encode(&host_id)
    );
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let mut params = vec![
        format!("instance_id={}", urlencoding::encode(&instance_id)),
        format!("limit={limit}"),
    ];
    let wait_ms = wait_ms.map(|value| value.clamp(0, 25_000));
    if let Some(wait_ms) = wait_ms {
        params.push(format!("wait_ms={wait_ms}"));
    }
    url.push_str(&format!("?{}", params.join("&")));
    let resp = clark_http_client()?
        .get(url)
        .timeout(Duration::from_millis(
            wait_ms.unwrap_or(0) as u64 + COMMAND_POLL_TIMEOUT_SLACK_MS,
        ))
        .bearer_auth(&access.token)
        .send()
        .await
        .map_err(|e| format!("Clark Code command poll request failed: {e}"))?;
    let mut value = read_json_or_err(resp, "Clark Code command poll").await?;
    if let Some(commands) = value.get_mut("commands").and_then(Value::as_array_mut) {
        for command in commands {
            let command_id = command
                .get("command_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let claim_instance_id = command
                .get("claim_instance_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let claim_token = command
                .as_object_mut()
                .and_then(|object| object.remove("claim_token"))
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            state
                .runtime_registry
                .store_command_claim(
                    account.clone(),
                    command_id,
                    host_id.clone(),
                    claim_instance_id,
                    claim_token,
                )
                .await?;
            trace_command_receipt(command, "desktop_claim");
        }
    }
    Ok(value)
}

/// Record a host-side command receipt so mobile can reconcile retries,
/// completion, and failures without guessing from snapshots.
#[tauri::command]
pub async fn desktop_code_command_ack(
    command_id: String,
    host_id: String,
    instance_id: String,
    status: String,
    response: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let account = crate::runtime_registry::AccountKey::new(access.owner_scope.clone())?;
    let claim_token = state
        .runtime_registry
        .command_claim(&account, &command_id, &host_id, &instance_id)
        .await?;
    let url = format!(
        "{}/api/desktop/code/commands/{}/ack",
        access.rest_base,
        urlencoding::encode(&command_id)
    );
    let resp = clark_http_client()?
        .post(url)
        .bearer_auth(&access.token)
        .json(&serde_json::json!({
            "host_id": host_id,
            "instance_id": instance_id,
            "claim_token": claim_token,
            "status": status,
            "response": response,
        }))
        .send()
        .await
        .map_err(|e| format!("Clark Code command ack request failed: {e}"))?;
    let mut value = read_json_or_err(resp, "Clark Code command ack").await?;
    if let Some(command) = value.get_mut("command") {
        if let Some(command) = command.as_object_mut() {
            command.remove("claim_token");
        }
        trace_command_receipt(command, "desktop_ack");
    }
    if matches!(status.as_str(), "completed" | "failed" | "rejected") {
        state
            .runtime_registry
            .remove_command_claim(&account, &command_id)
            .await;
    }
    Ok(value)
}

/// Fetch one command-bound attachment through a fresh authenticated lease.
/// The presigned object URL never enters the WebView or durable command body.
#[tauri::command]
pub async fn desktop_code_attachment_download(
    command_id: String,
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<DownloadedCodeRemoteAttachment, String> {
    let access = current_cloud_access(state.inner()).await?;
    code_attachment_download(access, command_id, attachment_id).await
}

async fn code_attachment_download(
    access: CloudAccess,
    command_id: String,
    attachment_id: String,
) -> Result<DownloadedCodeRemoteAttachment, String> {
    let lease_url = format!(
        "{}/api/desktop/code/commands/{}/attachments/{}",
        access.rest_base,
        urlencoding::encode(&command_id),
        urlencoding::encode(&attachment_id),
    );
    let client = clark_http_client()?;
    let lease_response = client
        .get(lease_url)
        .timeout(HOST_REQUEST_TIMEOUT)
        .bearer_auth(access.token)
        .send()
        .await
        .map_err(|error| format!("Clark Code attachment lease request failed: {error}"))?;
    let lease_value = read_json_or_err(lease_response, "Clark Code attachment lease").await?;
    let lease: CodeRemoteAttachmentDownloadLease = serde_json::from_value(lease_value)
        .map_err(|error| format!("Clark Code attachment lease was malformed: {error}"))?;
    if lease.size_bytes == 0 || lease.size_bytes as usize > MAX_CODE_REMOTE_ATTACHMENT_BYTES {
        return Err("Clark Code attachment size is outside the supported range".into());
    }
    let object_response = client
        .get(&lease.url)
        .timeout(ATTACHMENT_DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("Clark Code attachment download failed: {error}"))?;
    if !object_response.status().is_success() {
        return Err(format!(
            "Clark Code attachment download failed with status {}",
            object_response.status()
        ));
    }
    let bytes = object_response
        .bytes()
        .await
        .map_err(|error| format!("Clark Code attachment body could not be read: {error}"))?;
    if bytes.len() != lease.size_bytes as usize {
        return Err(format!(
            "Clark Code attachment size mismatch: expected {}, got {}",
            lease.size_bytes,
            bytes.len()
        ));
    }
    Ok(DownloadedCodeRemoteAttachment {
        filename: lease.filename,
        content_type: lease.content_type,
        size_bytes: bytes.len(),
        data_base64: BASE64.encode(bytes),
    })
}

#[tauri::command]
pub async fn desktop_code_repository_sync(
    batch: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!("{}/api/desktop/code/repositories", access.rest_base);
    let resp = clark_http_client()?
        .post(url)
        .bearer_auth(access.token)
        .json(&batch)
        .send()
        .await
        .map_err(|e| format!("Clark Code repository sync request failed: {e}"))?;
    read_json_or_err(resp, "Clark Code repository sync").await
}

/// Return only organizations where organizational memory is both enabled and
/// accessible to the signed-in user. The desktop still requires a separate,
/// per-repository local opt-in before contributing anything.
#[tauri::command]
pub async fn desktop_organization_knowledge_status(
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!("{}/api/desktop/organization-knowledge", access.rest_base);
    let resp = clark_http_client()?
        .get(url)
        .bearer_auth(access.token)
        .send()
        .await
        .map_err(|e| format!("Organization knowledge status request failed: {e}"))?;
    read_json_or_err(resp, "Organization knowledge status").await
}

/// Contribute one already-bounded repository-history batch to an explicitly
/// selected organization. The backend also performs the user's personal
/// repository sync so the client never has to upload the same batch twice.
#[tauri::command]
pub async fn desktop_organization_repository_sync(
    organization_id: String,
    host_id: String,
    batch: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!(
        "{}/api/desktop/organization-knowledge/repositories",
        access.rest_base
    );
    let resp = clark_http_client()?
        .post(url)
        .bearer_auth(access.token)
        .json(&serde_json::json!({
            "organization_id": organization_id,
            "host_id": host_id,
            "batch": batch,
        }))
        .send()
        .await
        .map_err(|e| format!("Organization repository sync request failed: {e}"))?;
    read_json_or_err(resp, "Organization repository sync").await
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{code_attachment_download, code_host_upsert};
    use crate::commands::CloudAccess;

    #[tokio::test]
    async fn host_upsert_reconnects_after_a_broken_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8_192];
            let _ = second.read(&mut request).await.unwrap();
            second
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
        });

        let access = |endpoint: String| {
            CloudAccess::for_test(
                endpoint.trim_end_matches("/ws").into(),
                "account-test".into(),
                "token".into(),
            )
        };
        let first = code_host_upsert(
            access(endpoint.clone()),
            "host".into(),
            "Desktop".into(),
            "macOS".into(),
            "arm64".into(),
            "test".into(),
            2,
            serde_json::json!(["send_message"]),
            serde_json::json!([]),
        )
        .await;
        assert!(first.is_err());

        let second = code_host_upsert(
            access(endpoint),
            "host".into(),
            "Desktop".into(),
            "macOS".into(),
            "arm64".into(),
            "test".into(),
            2,
            serde_json::json!(["send_message"]),
            serde_json::json!([]),
        )
        .await;
        assert_eq!(second.unwrap(), serde_json::json!({}));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn attachment_download_uses_authenticated_lease_and_checks_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let endpoint = format!("{base_url}/ws");
        let object_url = format!("{base_url}/object");
        let server = tokio::spawn(async move {
            let (mut lease_socket, _) = listener.accept().await.unwrap();
            let mut lease_request = vec![0; 8_192];
            let lease_size = lease_socket.read(&mut lease_request).await.unwrap();
            let lease_request = String::from_utf8_lossy(&lease_request[..lease_size]);
            assert!(lease_request
                .contains("GET /api/desktop/code/commands/cmd-1/attachments/codeatt-1"));
            assert!(lease_request.contains("authorization: Bearer token"));
            let lease_body = serde_json::json!({
                "url": object_url,
                "filename": "note.txt",
                "content_type": "text/plain",
                "size_bytes": 4,
            })
            .to_string();
            lease_socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        lease_body.len(), lease_body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let (mut object_socket, _) = listener.accept().await.unwrap();
            let mut object_request = vec![0; 8_192];
            let object_size = object_socket.read(&mut object_request).await.unwrap();
            let object_request = String::from_utf8_lossy(&object_request[..object_size]);
            assert!(object_request.contains("GET /object"));
            object_socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\nconnection: close\r\n\r\ntest")
                .await
                .unwrap();
        });

        let downloaded = code_attachment_download(
            CloudAccess::for_test(
                endpoint.trim_end_matches("/ws").into(),
                "account-test".into(),
                "token".into(),
            ),
            "cmd-1".into(),
            "codeatt-1".into(),
        )
        .await
        .unwrap();
        assert_eq!(downloaded.filename, "note.txt");
        assert_eq!(downloaded.content_type, "text/plain");
        assert_eq!(downloaded.size_bytes, 4);
        assert_eq!(downloaded.data_base64, "dGVzdA==");
        server.await.unwrap();
    }
}
