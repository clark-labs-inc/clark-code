use super::cloud_authority::current_cloud_access;
use super::*;

// ---------------------------------------------------------------------------
// Desktop conversation cloud sync
//
// The local coding agent's transcripts are stored on Clark via the desktop
// conversation API (`/api/desktop/conversations`). Calls run host-side (reqwest)
// so they aren't subject to WebView CORS, and authenticate with the user's Clark
// JWT. The gateway serves both `/ws` and `/api/...` on one host, so the REST base
// is the WS endpoint with an http(s) scheme and the `/ws` suffix dropped.

pub(crate) async fn read_json_or_err(resp: reqwest::Response, what: &str) -> Result<Value, String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{what} failed ({status}): {text}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("{what}: invalid response: {e}"))
}

/// Probe MCP servers — connect each, list its tools, return status — then drop
/// them. A stateless "test connection" for the MCP settings UI.
#[tauri::command]
pub async fn clark_mcp_probe(
    mut servers: Vec<provider_local::McpServerConfig>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::McpStatus>, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let owner_scope = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .ok_or("Clark must be signed in before testing MCP servers")?;
    hydrate_mcp_servers(&mut servers, &owner_scope, state.inner()).await?;
    Ok(provider_local::probe_mcp_servers(&servers).await)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCredentialUpdate {
    id: String,
    environment: std::collections::HashMap<String, String>,
}

/// Atomically replace this signed-in account's MCP credential set. Empty
/// values retain an existing value, allowing the WebView to edit descriptors
/// without receiving a stored secret back from native code.
#[tauri::command]
pub async fn clark_mcp_credentials_sync(
    servers: Vec<McpCredentialUpdate>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let owner_scope = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .ok_or("Clark must be signed in before saving MCP credentials")?;
    let server_count = servers.len();
    let environments = servers
        .into_iter()
        .map(|server| (server.id, server.environment))
        .collect::<std::collections::HashMap<_, _>>();
    if environments.len() != server_count {
        return Err("MCP server ids must be unique".into());
    }
    state
        .credentials
        .sync_mcp_environment(&owner_scope, environments)
        .await
}

#[tauri::command]
pub async fn clark_repository_inspect(
    cwd: String,
) -> Result<Option<provider_local::RepositoryIdentity>, String> {
    provider_local::inspect_repository(&provider_local::LocalExecutor, std::path::Path::new(&cwd))
        .await
}

#[tauri::command]
pub async fn clark_repository_discover(
    cwd: String,
) -> Result<Vec<provider_local::RepositoryIdentity>, String> {
    provider_local::discover_repositories(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
    )
    .await
}

#[tauri::command]
pub async fn clark_repository_history(
    cwd: String,
    offset: usize,
    limit: usize,
) -> Result<Option<provider_local::GitHistoryBatch>, String> {
    provider_local::load_git_history(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
        offset,
        limit,
    )
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeCredentialStatus {
    pub ready: bool,
}

/// Ensure this signed-in account has a Clark Code credential in Clark's
/// app-owned encrypted file. The key never enters an IPC result, WebView
/// persistence, logs, or an operating-system credential store.
#[tauri::command]
pub async fn clark_provision_code_key(
    state: State<'_, AppState>,
) -> Result<CodeCredentialStatus, String> {
    // The native access lease is both the single-flight key provisioner and
    // account-switch barrier. Sign-out waits, then deletes the generation.
    let access = current_cloud_access(state.inner()).await?;
    if state
        .credentials
        .code_key(&access.owner_scope)
        .await?
        .is_some()
    {
        return Ok(CodeCredentialStatus { ready: true });
    }
    let url = format!("{}/api/platform/api-keys", access.rest_base);
    let resp = clark_http_client()?
        .post(url)
        .bearer_auth(access.token)
        .json(&serde_json::json!({
            "name": "Clark Code (Desktop)",
            "purpose": "clark_code_desktop",
        }))
        .send()
        .await
        .map_err(|e| format!("key provision request failed: {e}"))?;
    let v = read_json_or_err(resp, "provision Clark Code key").await?;
    let key = v
        .get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Clark did not return an API key".to_string())?;
    state
        .credentials
        .set_code_key(&access.owner_scope, key)
        .await?;
    Ok(CodeCredentialStatus { ready: true })
}

/// Fetch the signed-in user's billing summary (subscription, plan, credits,
/// recent ledger) — `GET /api/billing/me`. Returned verbatim to the UI.
#[tauri::command]
pub async fn clark_billing_me(state: State<'_, AppState>) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!("{}/api/billing/me", access.rest_base);
    let resp = clark_http_client()?
        .get(url)
        .bearer_auth(access.token)
        .send()
        .await
        .map_err(|e| format!("billing request failed: {e}"))?;
    read_json_or_err(resp, "billing").await
}

/// Create (or fetch the existing) public share for a synced conversation.
/// Returns `{ share_token, share_url }`.
#[tauri::command]
pub async fn desktop_conv_share(id: String, state: State<'_, AppState>) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!(
        "{}/api/desktop/conversations/{}/share",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .post(url)
        .bearer_auth(access.token)
        .send()
        .await
        .map_err(|e| format!("share request failed: {e}"))?;
    read_json_or_err(resp, "share conversation").await
}

/// Revoke the public share for a conversation (idempotent).
#[tauri::command]
pub async fn desktop_conv_unshare(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let access = current_cloud_access(state.inner()).await?;
    let url = format!(
        "{}/api/desktop/conversations/{}/share",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .delete(url)
        .bearer_auth(access.token)
        .send()
        .await
        .map_err(|e| format!("unshare request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("unshare failed ({status}): {text}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn desktop_conv_delete(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    super::cloud_conversations::desktop_conversation_client(&access.rest_base, &token)?
        .delete(&id)
        .await
        .map_err(|error| error.to_string())?;
    crate::trajectory::delete_conversation(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        id,
    )
    .await
}

/// Toggle a desktop conversation's archived flag in the cloud (a snapshot `put`
/// never changes it, so this is the only path that does). Returns the updated
/// summary.
#[tauri::command]
pub async fn desktop_conv_set_archived(
    app: AppHandle,
    id: String,
    archived: bool,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let summary =
        super::cloud_conversations::desktop_conversation_client(&access.rest_base, &token)?
            .set_archived(&id, archived)
            .await
            .map_err(|error| error.to_string())?;
    crate::trajectory::set_archived(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        id,
        archived,
    )
    .await?;
    serde_json::to_value(summary)
        .map_err(|error| format!("desktop archive serialization failed: {error}"))
}
