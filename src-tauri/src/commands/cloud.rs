use super::cloud_authority::current_account_access;
use super::*;

// ---------------------------------------------------------------------------
// Durable conversation commands. Product transport is delegated through the
// opaque request bridge; this module owns only local recovery and projection.

/// Probe MCP servers — connect each, list its tools, return status — then drop
/// them. A stateless "test connection" for the MCP settings UI.
#[tauri::command]
pub async fn mcp_probe(
    mut servers: Vec<provider_local::McpServerConfig>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::McpStatus>, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let owner_scope = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .unwrap_or_else(|| "local".to_string());
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
pub async fn mcp_credentials_sync(
    servers: Vec<McpCredentialUpdate>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let owner_scope = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .unwrap_or_else(|| "local".to_string());
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
pub async fn repository_inspect(
    cwd: String,
) -> Result<Option<provider_local::RepositoryIdentity>, String> {
    provider_local::inspect_repository(&provider_local::LocalExecutor, std::path::Path::new(&cwd))
        .await
}

#[tauri::command]
pub async fn repository_discover(
    cwd: String,
) -> Result<Vec<provider_local::RepositoryIdentity>, String> {
    provider_local::discover_repositories(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
    )
    .await
}

#[tauri::command]
pub async fn repository_history(
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

#[tauri::command]
pub async fn desktop_conv_delete(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access = current_account_access(state.inner()).await?;
    match super::cloud_conversations::product_cloud_request(
        "conversation.delete",
        serde_json::json!({ "id": id }),
        &app,
        state.inner(),
    )
    .await?
    {
        super::cloud_conversations::ProductCloudOutcome::Ok(_) => {}
        super::cloud_conversations::ProductCloudOutcome::Unauthorized(error)
        | super::cloud_conversations::ProductCloudOutcome::NotFound(error)
        | super::cloud_conversations::ProductCloudOutcome::Conflict(error)
        | super::cloud_conversations::ProductCloudOutcome::Unavailable(error)
        | super::cloud_conversations::ProductCloudOutcome::Rejected(error) => return Err(error),
    }
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
    let access = current_account_access(state.inner()).await?;
    let summary = match super::cloud_conversations::product_cloud_request(
        "conversation.archive",
        serde_json::json!({ "id": id, "archived": archived }),
        &app,
        state.inner(),
    )
    .await?
    {
        super::cloud_conversations::ProductCloudOutcome::Ok(summary) => summary,
        super::cloud_conversations::ProductCloudOutcome::Unauthorized(error)
        | super::cloud_conversations::ProductCloudOutcome::NotFound(error)
        | super::cloud_conversations::ProductCloudOutcome::Conflict(error)
        | super::cloud_conversations::ProductCloudOutcome::Unavailable(error)
        | super::cloud_conversations::ProductCloudOutcome::Rejected(error) => return Err(error),
    };
    crate::trajectory::set_archived(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        id,
        archived,
    )
    .await?;
    Ok(summary)
}
