//! Clark Code mobile remote-control IPC bridge.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::commands::{clark_http_client, clark_rest_base, read_json_or_err};

const HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const ATTACHMENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_POLL_TIMEOUT_SLACK_MS: u64 = 10_000;
const MAX_CODE_REMOTE_ATTACHMENT_BYTES: usize = 12 * 1024 * 1024;

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
    endpoint: String,
    token: String,
    host_id: String,
    display_name: String,
    os_name: String,
    arch: String,
    app_version: String,
    projects: Value,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/code/hosts/{}",
        clark_rest_base(&endpoint),
        urlencoding::encode(&host_id)
    );
    let resp = clark_http_client()?
        .put(url)
        .timeout(HOST_REQUEST_TIMEOUT)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "display_name": display_name,
            "os": os_name,
            "arch": arch,
            "app_version": app_version,
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
    endpoint: String,
    token: String,
    host_id: String,
    limit: Option<i64>,
    wait_ms: Option<i64>,
) -> Result<Value, String> {
    let mut url = format!(
        "{}/api/desktop/code/hosts/{}/commands",
        clark_rest_base(&endpoint),
        urlencoding::encode(&host_id)
    );
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let mut params = vec![format!("limit={limit}")];
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
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Clark Code command poll request failed: {e}"))?;
    read_json_or_err(resp, "Clark Code command poll").await
}

/// Record a host-side command receipt so mobile can reconcile retries,
/// completion, and failures without guessing from snapshots.
#[tauri::command]
pub async fn desktop_code_command_ack(
    endpoint: String,
    token: String,
    command_id: String,
    host_id: String,
    status: String,
    response: Value,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/code/commands/{}/ack",
        clark_rest_base(&endpoint),
        urlencoding::encode(&command_id)
    );
    let resp = clark_http_client()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "host_id": host_id,
            "status": status,
            "response": response,
        }))
        .send()
        .await
        .map_err(|e| format!("Clark Code command ack request failed: {e}"))?;
    read_json_or_err(resp, "Clark Code command ack").await
}

/// Fetch one command-bound attachment through a fresh authenticated lease.
/// The presigned object URL never enters the WebView or durable command body.
#[tauri::command]
pub async fn desktop_code_attachment_download(
    endpoint: String,
    token: String,
    command_id: String,
    attachment_id: String,
) -> Result<DownloadedCodeRemoteAttachment, String> {
    let lease_url = format!(
        "{}/api/desktop/code/commands/{}/attachments/{}",
        clark_rest_base(&endpoint),
        urlencoding::encode(&command_id),
        urlencoding::encode(&attachment_id),
    );
    let client = clark_http_client()?;
    let lease_response = client
        .get(lease_url)
        .timeout(HOST_REQUEST_TIMEOUT)
        .header("Authorization", format!("Bearer {token}"))
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
    endpoint: String,
    token: String,
    batch: Value,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/code/repositories",
        clark_rest_base(&endpoint)
    );
    let resp = clark_http_client()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
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
    endpoint: String,
    token: String,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/organization-knowledge",
        clark_rest_base(&endpoint)
    );
    let resp = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
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
    endpoint: String,
    token: String,
    organization_id: String,
    host_id: String,
    batch: Value,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/organization-knowledge/repositories",
        clark_rest_base(&endpoint)
    );
    let resp = clark_http_client()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
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

    use super::{desktop_code_attachment_download, desktop_code_host_upsert};

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

        let first = desktop_code_host_upsert(
            endpoint.clone(),
            "token".into(),
            "host".into(),
            "Desktop".into(),
            "macOS".into(),
            "arm64".into(),
            "test".into(),
            serde_json::json!([]),
        )
        .await;
        assert!(first.is_err());

        let second = desktop_code_host_upsert(
            endpoint,
            "token".into(),
            "host".into(),
            "Desktop".into(),
            "macOS".into(),
            "arm64".into(),
            "test".into(),
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

        let downloaded = desktop_code_attachment_download(
            endpoint,
            "token".into(),
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
