//! Tauri command surface — the IPC boundary the web UI calls via `invoke`.
//! These mirror the `agent_core::Provider` trait and drive the live provider.

use agent_core::{
    apply, ClientResponse, ContentBlock, PendingUpload, PromptInput, Provider, ProviderConfig,
    RunId, SessionId, SessionOptions, Snapshot,
};
use agent_core::{AgentEvent, Role};
use futures::StreamExt;
use provider_acp::AcpProvider;
use provider_clark::ClarkProvider;
use provider_local::LocalAgentProvider;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::{builtin_providers, AppState, ProviderInfo};

/// Synthetic run id used to attribute the user's own message in the timeline.
const USER_RUN: &str = "user";

/// Construct a provider instance by id.
fn make_provider(id: &str) -> Result<Box<dyn Provider>, String> {
    match id {
        "acp" => Ok(Box::new(AcpProvider::new())),
        "clark" => Ok(Box::new(ClarkProvider::new())),
        "local" => Ok(Box::new(LocalAgentProvider::new())),
        other => Err(format!("unknown provider: {other}")),
    }
}

#[tauri::command]
pub fn provider_list() -> Vec<ProviderInfo> {
    builtin_providers()
}

#[tauri::command]
pub async fn provider_connect(
    provider_id: String,
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(provider = %provider_id, "connecting");
    let mut provider = make_provider(&provider_id)?;
    provider.connect(config).await.map_err(|e| e.to_string())?;
    state.session.lock().await.provider = Some(provider);
    Ok(())
}

#[tauri::command]
pub async fn session_new(
    provider_id: String,
    options: SessionOptions,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    tracing::info!(provider = %provider_id, "session_new");
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    let session = provider
        .new_session(options)
        .await
        .map_err(|e| e.to_string())?;
    let mut snapshot = Snapshot::new();
    snapshot.session = Some(session.id.clone());
    s.snapshot = snapshot;
    s.session = Some(session.clone());
    let _ = app.emit("snapshot", &s.snapshot);
    serde_json::to_value(&session).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_load(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    tracing::info!(session = %id, "session_load");
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    let session = provider
        .load_session(SessionId::new(id))
        .await
        .map_err(|e| e.to_string())?;
    // The client restores the persisted transcript; start from a clean snapshot
    // bound to the resumed session so new turns append correctly.
    let mut snapshot = Snapshot::new();
    snapshot.session = Some(session.id.clone());
    s.snapshot = snapshot;
    s.session = Some(session.clone());
    let _ = app.emit("snapshot", &s.snapshot);
    serde_json::to_value(&session).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prompt(
    app: AppHandle,
    session_id: String,
    blocks: Vec<ContentBlock>,
    attachments: Vec<PendingUpload>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let sid = SessionId::new(session_id);

    // Show the user's message immediately (providers don't reliably echo it),
    // then lock the provider to obtain the run's event stream and release.
    let mut stream = {
        let mut s = state.session.lock().await;
        for block in &blocks {
            apply(
                &mut s.snapshot,
                &AgentEvent::MessageChunk {
                    run: RunId::new(USER_RUN),
                    role: Role::User,
                    delta: block.clone(),
                },
            );
        }
        let _ = app.emit("snapshot", &s.snapshot);

        let provider = s.provider.as_mut().ok_or("connect a provider first")?;
        provider
            .prompt(
                &sid,
                PromptInput {
                    blocks,
                    attachments,
                },
            )
            .await
            .map_err(|e| e.to_string())?
    };

    // Fold events into the shared snapshot and push each update to the webview.
    let host = state.session.clone();
    tokio::spawn(async move {
        while let Some(ev) = stream.next().await {
            let snapshot = {
                let mut s = host.lock().await;
                apply(&mut s.snapshot, &ev);
                s.snapshot.clone()
            };
            let _ = app.emit("snapshot", &snapshot);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel(
    session_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    provider
        .cancel(&SessionId::new(session_id), &RunId::new(run_id))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn respond(
    session_id: String,
    response: ClientResponse,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    provider
        .respond(&SessionId::new(session_id), response)
        .await
        .map_err(|e| e.to_string())
}

/// Extract a per-repository project memory via Clark's agentic Platform API and
/// write it to `<cwd>/.clark/memory/MEMORY.md`. Returns the memory text on
/// success. Uses the production Clark Platform API with the given `ck_live_` key.
#[tauri::command]
pub async fn local_extract_memory(
    cwd: String,
    api_key: String,
    model: Option<String>,
) -> Result<String, String> {
    provider_local::extract_repo_memory(
        std::path::Path::new(&cwd),
        provider_local::DEFAULT_BASE_URL,
        Some(api_key.as_str()),
        model
            .as_deref()
            .unwrap_or(provider_local::DEFAULT_RESEARCH_MODEL),
    )
    .await
}

/// One per-fact memory file, flattened for the UI.
#[derive(serde::Serialize)]
pub struct MemoryFactView {
    pub file: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub body: String,
}

/// Everything the memory viewer needs for one project folder.
#[derive(serde::Serialize)]
pub struct MemoryOverview {
    /// Absolute path to `<cwd>/.clark/memory`.
    pub dir: String,
    /// Whether a project-memory index (`MEMORY.md`) has been written.
    pub exists: bool,
    /// Contents of the always-loaded `MEMORY.md` index, if present.
    pub index: Option<String>,
    /// Per-fact memory files (newest first).
    pub facts: Vec<MemoryFactView>,
}

/// List the per-repository memory for `cwd`: the `MEMORY.md` index plus any
/// per-fact files under `<cwd>/.clark/memory/`. Read-only; never writes.
#[tauri::command]
pub async fn local_list_memory(cwd: String) -> Result<MemoryOverview, String> {
    if cwd.trim().is_empty() {
        return Err("choose a project folder first".into());
    }
    let root = std::path::Path::new(&cwd);
    let facts = provider_local::load_facts(root)
        .into_iter()
        .map(|f| MemoryFactView {
            file: f.header.file,
            name: f.header.name,
            description: f.header.description,
            kind: f.header.kind.map(|k| k.label().to_string()),
            body: f.body,
        })
        .collect();
    Ok(MemoryOverview {
        dir: provider_local::memory_dir(root)
            .to_string_lossy()
            .to_string(),
        exists: provider_local::has_memory(root),
        index: provider_local::load_index(root),
        facts,
    })
}

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
fn clark_rest_base(endpoint: &str) -> String {
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

fn clark_http_client() -> Result<reqwest::Client, String> {
    Ok(CLOUD_HTTP.clone())
}

async fn read_json_or_err(resp: reqwest::Response, what: &str) -> Result<Value, String> {
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

/// List the signed-in user's desktop conversations (metadata only).
#[tauri::command]
pub async fn desktop_conv_list(endpoint: String, token: String) -> Result<Value, String> {
    let url = format!("{}/api/desktop/conversations", clark_rest_base(&endpoint));
    let resp = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("desktop list request failed: {e}"))?;
    read_json_or_err(resp, "desktop list").await
}

/// Fetch one desktop conversation including its full snapshot blob.
#[tauri::command]
pub async fn desktop_conv_get(
    endpoint: String,
    token: String,
    id: String,
) -> Result<Value, String> {
    let url = format!(
        "{}/api/desktop/conversations/{}",
        clark_rest_base(&endpoint),
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("desktop get request failed: {e}"))?;
    read_json_or_err(resp, "desktop get").await
}

/// Insert or replace a desktop conversation snapshot.
#[tauri::command]
pub async fn desktop_conv_put(
    endpoint: String,
    token: String,
    id: String,
    title: String,
    provider: String,
    project: Option<String>,
    snapshot: Value,
) -> Result<Value, String> {
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
            "snapshot": snapshot,
        }))
        .send()
        .await
        .map_err(|e| format!("desktop put request failed: {e}"))?;
    read_json_or_err(resp, "desktop put").await
}

/// Probe MCP servers — connect each, list its tools, return status — then drop
/// them. A stateless "test connection" for the MCP settings UI.
#[tauri::command]
pub async fn clark_mcp_probe(
    servers: Vec<provider_local::McpServerConfig>,
) -> Result<Vec<provider_local::McpStatus>, String> {
    Ok(provider_local::probe_mcp_servers(&servers).await)
}

/// Restore the project's working tree to a pre-run checkpoint (one-click undo).
/// `sha` is the run's `checkpoint` handle. Runs git off the UI thread.
#[tauri::command]
pub async fn clark_checkpoint_restore(cwd: String, sha: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        provider_local::restore_checkpoint(std::path::Path::new(&cwd), &sha)
    })
    .await
    .map_err(|e| format!("restore task failed: {e}"))?
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
        .json(&serde_json::json!({ "name": "Clark Code (Desktop)" }))
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

/// Delete a desktop conversation from the cloud.
#[tauri::command]
pub async fn desktop_conv_delete(
    endpoint: String,
    token: String,
    id: String,
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
    Ok(())
}
