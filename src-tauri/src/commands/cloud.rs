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
/// browser CORS against the Clark auth origin. No secrets are involved: the
/// Google ID token is short-lived and the call only reads back Clark's own JWT.
#[tauri::command]
pub async fn clark_exchange_google_idtoken(
    auth_origin: String,
    id_token: String,
) -> Result<GoogleAuthResult, String> {
    let base = auth_origin.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .cookie_store(true)
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

    Ok(GoogleAuthResult {
        token,
        id: str_field(&user, "id").unwrap_or_default(),
        email: str_field(&user, "email").unwrap_or_default(),
        name: str_field(&user, "name"),
        image: str_field(&user, "image"),
    })
}

// ---------------------------------------------------------------------------
// Desktop conversation cloud sync
//
// The local coding agent's transcripts are stored on Clark via the desktop
// conversation API (`/api/desktop/conversations`). Calls run host-side (reqwest)
// so they aren't subject to WebView CORS, and authenticate with the user's Clark
// JWT. The gateway serves both `/ws` and `/api/...` on one host, so the REST base
// is the WS endpoint with an http(s) scheme and the `/ws` suffix dropped.

/// Derive the HTTPS REST base from the gateway WS endpoint.
pub(crate) fn clark_rest_base(endpoint: &str) -> String {
    let mut base = endpoint.trim().to_string();
    if let Some(rest) = base.strip_prefix("wss://") {
        base = format!("https://{rest}");
    } else if let Some(rest) = base.strip_prefix("ws://") {
        base = format!("http://{rest}");
    }
    let base = base.trim_end_matches('/');
    base.strip_suffix("/ws").unwrap_or(base).to_string()
}

/// Shared HTTP client for cloud sync. Built once and reused so connections stay
/// warm (HTTP keep-alive / HTTP/2): each desktop-conversation write is then a
/// single round-trip, not a fresh TLS handshake — that per-request rebuild was
/// what made the REST sync feel slow.
static CLOUD_HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("build cloud http client")
});

pub(crate) fn clark_http_client() -> Result<reqwest::Client, String> {
    Ok(CLOUD_HTTP.clone())
}

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

/// List the signed-in user's desktop conversations (metadata only). The cloud
/// response is authoritative; the account-scoped SQLite cache only fills rows
/// that have not reached the cloud yet or keeps history available offline.
#[tauri::command]
pub async fn desktop_conv_list(
    app: AppHandle,
    endpoint: String,
    token: String,
    owner_scope: String,
) -> Result<Value, String> {
    let url = format!("{}/api/desktop/conversations", clark_rest_base(&endpoint));
    let cloud = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;
    let (rows, cloud_available) = match cloud {
        Ok(response) => match read_json_or_err(response, "desktop list").await {
            Ok(value) => (value.as_array().cloned().unwrap_or_default(), true),
            Err(error) => {
                tracing::warn!(%error, "desktop cloud list unavailable; using local acknowledged cache");
                (Vec::new(), false)
            }
        },
        Err(error) => {
            tracing::warn!(%error, "desktop cloud list unavailable; using local acknowledged cache");
            (Vec::new(), false)
        }
    };
    let merged = crate::trajectory::merge_local_summaries(
        crate::trajectory::outbox_path(&app)?,
        owner_scope,
        rows,
        cloud_available,
    )
    .await?;
    Ok(Value::Array(merged))
}

/// Fetch one desktop conversation including its full snapshot blob.
#[tauri::command]
pub async fn desktop_conv_get(
    app: AppHandle,
    endpoint: String,
    token: String,
    id: String,
    owner_scope: String,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/conversations/{}",
        clark_rest_base(&endpoint),
        urlencoding::encode(&id)
    );
    let cloud = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;
    let cloud_detail = match cloud {
        Ok(response) => read_json_or_err(response, "desktop get").await.ok(),
        Err(error) => {
            tracing::warn!(%error, conversation_id = %id, "desktop cloud get unavailable; using local acknowledged cache");
            None
        }
    };
    let cloud_snapshot = cloud_detail.as_ref().and_then(|detail| {
        let snapshot = serde_json::from_value(detail.get("snapshot")?.clone()).ok()?;
        let rev = detail
            .get("rev")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        Some((snapshot, rev))
    });
    let recovered = crate::trajectory::recover_snapshot(
        crate::trajectory::outbox_path(&app)?,
        owner_scope,
        id.clone(),
        cloud_snapshot,
    )
    .await?;
    match (cloud_detail, recovered) {
        (Some(mut detail), Some(recovered)) => {
            let mut snapshot =
                serde_json::to_value(recovered.snapshot).map_err(|e| e.to_string())?;
            if recovered.pending {
                snapshot["sync_pending"] = true.into();
            }
            detail["snapshot"] = snapshot;
            detail["syncPending"] = recovered.pending.into();
            Ok(detail)
        }
        (Some(detail), None) => Ok(detail),
        (None, Some(recovered)) => {
            let mut snapshot =
                serde_json::to_value(recovered.snapshot).map_err(|e| e.to_string())?;
            if recovered.pending {
                snapshot["sync_pending"] = true.into();
            }
            Ok(serde_json::json!({
                "id": id,
                "snapshot": snapshot,
                "syncPending": recovered.pending,
            }))
        }
        (None, None) => Err(format!(
            "desktop conversation {id} is unavailable locally and in Clark cloud"
        )),
    }
}

/// Insert or replace a desktop conversation snapshot.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn desktop_conv_put(
    app: AppHandle,
    endpoint: String,
    token: String,
    id: String,
    title: String,
    provider: String,
    project: Option<String>,
    repository_fingerprint: Option<String>,
    remote_host: Option<String>,
    mode: Option<String>,
    title_locked: bool,
    rev: i64,
    mut snapshot: Value,
    status: Option<String>,
    owner_scope: String,
    base_rev: Option<i64>,
    mutation_id: Option<String>,
) -> Result<Value, String> {
    let local_live = status.as_deref() == Some("running");
    let checkpoint_seq = snapshot
        .get("history_checkpoint")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    // Do not let the cloud read model overtake the append-only history it
    // represents. This command already runs in the background, so waiting for
    // the exact covered prefix does not block local rendering or offline work.
    crate::trajectory::wait_for_acknowledged_prefix(
        crate::trajectory::outbox_path(&app)?,
        owner_scope.clone(),
        id.clone(),
        checkpoint_seq,
        std::time::Duration::from_secs(10),
    )
    .await?;
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("history_checkpoint");
        object.remove("sync_pending");
    }
    let checkpoint_snapshot = snapshot.clone();
    let checkpoint_metadata = serde_json::json!({
        "id": id,
        "title": title,
        "provider": provider,
        "project": project,
        "repositoryFingerprint": repository_fingerprint,
        "remoteHost": remote_host,
        "mode": mode,
        "titleLocked": title_locked,
        "rev": rev,
        "archived": false,
    });
    let url = format!(
        "{}/api/desktop/conversations/{}",
        clark_rest_base(&endpoint),
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .put(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "provider": provider,
            "project": project,
            "repositoryFingerprint": repository_fingerprint,
            "remoteHost": remote_host,
            "mode": mode,
            "titleLocked": title_locked,
            "rev": rev,
            "snapshot": snapshot,
            "status": status,
            "baseRev": base_rev,
            "mutationId": mutation_id,
        }))
        .send()
        .await
        .map_err(|e| format!("desktop put request failed: {e}"))?;
    if resp.status() == reqwest::StatusCode::CONFLICT {
        let detail = resp.text().await.unwrap_or_default();
        crate::trajectory::quarantine_snapshot_branch(
            crate::trajectory::outbox_path(&app)?,
            owner_scope,
            id,
        )
        .await?;
        return Err(format!("desktop put failed (409 Conflict): {detail}"));
    }
    let summary = read_json_or_err(resp, "desktop put").await?;
    let stored_rev = summary
        .get("rev")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if stored_rev > rev {
        return Err(format!(
            "cloud_conflict: Clark cloud revision {stored_rev} is newer than local revision {rev}"
        ));
    }
    let typed_snapshot: Snapshot = serde_json::from_value(checkpoint_snapshot)
        .map_err(|error| format!("checkpoint desktop snapshot: {error}"))?;
    crate::trajectory::checkpoint_snapshot(
        crate::trajectory::outbox_path(&app)?,
        owner_scope,
        id.clone(),
        checkpoint_metadata,
        typed_snapshot,
        stored_rev,
        checkpoint_seq,
        local_live,
    )
    .await?;
    Ok(summary)
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
    let url = format!("{}/api/platform/api-keys", clark_rest_base(&endpoint));
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
    let url = format!("{}/api/billing/me", clark_rest_base(&endpoint));
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
        clark_rest_base(&endpoint),
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
        clark_rest_base(&endpoint),
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
    owner_scope: String,
) -> Result<(), String> {
    let url = format!(
        "{}/api/desktop/conversations/{}",
        clark_rest_base(&endpoint),
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
    crate::trajectory::delete_conversation(crate::trajectory::outbox_path(&app)?, owner_scope, id)
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
    owner_scope: String,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/conversations/{}",
        clark_rest_base(&endpoint),
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
        owner_scope,
        id,
        archived,
    )
    .await?;
    Ok(summary)
}
