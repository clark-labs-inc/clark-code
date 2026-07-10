//! Clark Code mobile remote-control IPC bridge.

use serde_json::Value;

use crate::commands::{clark_http_client, clark_rest_base, read_json_or_err};

/// Register or heartbeat this Clark Code host and publish the projects mobile
/// is allowed to start. Projects are server-validated before any command can
/// target them.
#[tauri::command]
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
    if let Some(wait_ms) = wait_ms {
        params.push(format!("wait_ms={}", wait_ms.clamp(0, 25_000)));
    }
    url.push_str(&format!("?{}", params.join("&")));
    let resp = clark_http_client()?
        .get(url)
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
