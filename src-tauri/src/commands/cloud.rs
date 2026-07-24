use super::cloud_authority::{
    clark_auth_base, clear_cloud_authority, install_cloud_authority, jwt_subject,
    refresh_cloud_authority, require_cloud_access,
};
use super::*;

/// Result of exchanging a Google ID token for a Clark session.
#[derive(serde::Serialize)]
pub struct GoogleAuthResult {
    /// Clark bearer JWT for the gateway WebSocket handshake.
    pub token: String,
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub image: Option<String>,
}

/// Exchange a Google ID token (from `tauri-plugin-google-auth`) for a Clark
/// session via Better Auth, then fetch the bearer JWT the gateway expects.
///
/// Done host-side (reqwest) rather than in the WebView so it isn't subject to
/// browser CORS. The short-lived Google credential remains native and is sent
/// only to an exact Clark authentication origin.
#[tauri::command]
pub async fn clark_exchange_google_idtoken(
    auth_origin: String,
    id_token: String,
    state: State<'_, AppState>,
) -> Result<GoogleAuthResult, String> {
    let base = clark_auth_base(&auth_origin)?;
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    // 1. Trade the Google ID token for a Clark session (sets the session cookie
    //    on this client's jar).
    let signin = client
        .post(format!("{base}/api/auth/sign-in/social"))
        .json(&serde_json::json!({
            "provider": "google",
            "idToken": { "token": id_token },
        }))
        .send()
        .await
        .map_err(|e| format!("sign-in request failed: {e}"))?;
    if !signin.status().is_success() {
        let status = signin.status();
        let body = signin.text().await.unwrap_or_default();
        return Err(format!(
            "Clark rejected the Google sign-in ({status}): {body}"
        ));
    }
    let signin_body: Value = signin.json().await.unwrap_or(Value::Null);

    // Prefer the user echoed by sign-in; fall back to get-session if absent.
    let mut user = signin_body.get("user").cloned().unwrap_or(Value::Null);
    if user
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        if let Ok(resp) = client
            .get(format!("{base}/api/auth/get-session"))
            .send()
            .await
        {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(u) = body.get("user") {
                    user = u.clone();
                }
            }
        }
    }

    // 2. Fetch the bearer JWT the gateway validates on the WebSocket handshake.
    let token_resp = client
        .get(format!("{base}/api/auth/token"))
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    if !token_resp.status().is_success() {
        return Err(format!(
            "Clark token bootstrap failed ({})",
            token_resp.status()
        ));
    }
    let token_body: Value = token_resp.json().await.map_err(|e| e.to_string())?;
    let token = token_body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        return Err("Clark returned an empty session token".into());
    }

    let str_field = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let id = str_field(&user, "id")
        .ok_or_else(|| "Clark sign-in returned no account identity".to_string())?;
    let subject = jwt_subject(&token)?;
    if subject != id {
        return Err("Clark session identity did not match the signed-in account".into());
    }
    install_cloud_authority(state.inner(), base, token.clone(), subject).await;

    Ok(GoogleAuthResult {
        token,
        id,
        email: str_field(&user, "email").unwrap_or_default(),
        name: str_field(&user, "name"),
        image: str_field(&user, "image"),
    })
}

/// Rotate the native bearer used by existing trajectory clients after the
/// WebView refreshes its Clark session. The subject must remain the account the
/// host already validated; a cross-account token is never allowed to inherit
/// that account's durable outbox.
#[tauri::command]
pub async fn clark_refresh_cloud_session(
    token: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    refresh_cloud_authority(state.inner(), &token).await
}

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
    servers: Vec<provider_local::McpServerConfig>,
) -> Result<Vec<provider_local::McpStatus>, String> {
    Ok(provider_local::probe_mcp_servers(&servers).await)
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

/// Provision (mint) a "Clark Code" platform API key for the signed-in user, so
/// the desktop never has to ask the user to paste one. Returns the full
/// `ck_live_…` key (shown only at creation — the caller persists it).
#[tauri::command]
pub async fn clark_provision_code_key(endpoint: String, token: String) -> Result<String, String> {
    let url = format!("{}/api/platform/api-keys", clark_rest_base(&endpoint)?);
    let resp = clark_http_client()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "name": "Clark Code (Desktop)",
            "purpose": "clark_code_desktop",
        }))
        .send()
        .await
        .map_err(|e| format!("key provision request failed: {e}"))?;
    let v = read_json_or_err(resp, "provision Clark Code key").await?;
    v.get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Clark did not return an API key".to_string())
}

/// Fetch the signed-in user's billing summary (subscription, plan, credits,
/// recent ledger) — `GET /api/billing/me`. Returned verbatim to the UI.
#[tauri::command]
pub async fn clark_billing_me(endpoint: String, token: String) -> Result<Value, String> {
    let url = format!("{}/api/billing/me", clark_rest_base(&endpoint)?);
    let resp = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("billing request failed: {e}"))?;
    read_json_or_err(resp, "billing").await
}

/// Create (or fetch the existing) public share for a synced conversation.
/// Returns `{ share_token, share_url }`.
#[tauri::command]
pub async fn desktop_conv_share(
    endpoint: String,
    token: String,
    id: String,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/conversations/{}/share",
        clark_rest_base(&endpoint)?,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("share request failed: {e}"))?;
    read_json_or_err(resp, "share conversation").await
}

/// Revoke the public share for a conversation (idempotent).
#[tauri::command]
pub async fn desktop_conv_unshare(
    endpoint: String,
    token: String,
    id: String,
) -> Result<(), String> {
    let url = format!(
        "{}/api/desktop/conversations/{}/share",
        clark_rest_base(&endpoint)?,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .delete(url)
        .header("Authorization", format!("Bearer {token}"))
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
    endpoint: String,
    token: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let url = format!(
        "{}/api/desktop/conversations/{}",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .delete(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("desktop delete request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("desktop delete failed ({status}): {text}"));
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
    endpoint: String,
    token: String,
    id: String,
    archived: bool,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let url = format!(
        "{}/api/desktop/conversations/{}",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .patch(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "archived": archived }))
        .send()
        .await
        .map_err(|e| format!("desktop archive request failed: {e}"))?;
    let summary = read_json_or_err(resp, "desktop archive").await?;
    crate::trajectory::set_archived(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        id,
        archived,
    )
    .await?;
    Ok(summary)
}

/// Clear native cloud credentials and cache authority during sign-out.
#[tauri::command]
pub async fn clark_clear_cloud_session(
    token: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    clear_cloud_authority(state.inner(), &token).await;
    Ok(())
}
