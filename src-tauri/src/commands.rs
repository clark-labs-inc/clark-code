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
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

use crate::ssh::{self, RemoteSpec};
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

/// What the frontend gets back after a remote project connects. The `remote`
/// block is spread verbatim into the local provider's connect `extra` (see
/// `LocalConfig`'s `RemoteTarget`), and `id` is used to disconnect later.
#[derive(Serialize)]
pub struct RemoteInfo {
    pub id: String,
    pub ws_url: String,
    pub token: String,
    pub cwd: String,
    pub arch: String,
}

/// Bring up a remote project: deploy + start `clark-exec-server` on `host`, open
/// the loopback tunnel, and return the `ws://` URL + token the local provider
/// uses as its remote executor. The connection is kept alive in host state under
/// the returned id until [`ssh_disconnect`].
#[tauri::command]
pub async fn ssh_connect(
    host: String,
    remote_root: String,
    local_binary: Option<String>,
    state: State<'_, AppState>,
) -> Result<RemoteInfo, String> {
    tracing::info!(%host, %remote_root, "ssh_connect");
    let spec = RemoteSpec {
        host,
        remote_root,
        // Empty/absent → rely on the CDN; a path is a dev override.
        local_binary: local_binary
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from),
    };
    let conn = ssh::connect(&spec).await?;
    let info = RemoteInfo {
        id: uuid::Uuid::new_v4().to_string(),
        ws_url: conn.ws_url.clone(),
        token: conn.token.clone(),
        cwd: conn.remote_root.clone(),
        arch: conn.arch.slug().to_string(),
    };
    state.remotes.lock().await.insert(info.id.clone(), conn);
    Ok(info)
}

/// Tear down a remote project: drop its `RemoteConn`, which kills the SSH
/// channels and, with them, the remote server + tunnel. Idempotent.
#[tauri::command]
pub async fn ssh_disconnect(id: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::info!(%id, "ssh_disconnect");
    state.remotes.lock().await.remove(&id);
    Ok(())
}

/// Read-only "test connection": reach `host` and report its architecture + home,
/// without deploying or tunneling. Backs the SSH-host settings test button.
#[tauri::command]
pub async fn ssh_probe(host: String) -> Result<ssh::Probe, String> {
    tracing::info!(%host, "ssh_probe");
    ssh::probe(&host).await
}

/// What Clark can migrate from an existing Claude Code setup in `cwd`.
#[derive(serde::Serialize)]
pub struct ClaudeDiscovery {
    /// MCP servers found in `.mcp.json` / `~/.claude.json` / `.claude/settings*`.
    pub mcp: Vec<provider_local::McpServerConfig>,
    /// Skills found in `.claude/skills` (project + personal).
    pub skills: Vec<provider_local::ClaudeSkill>,
}

/// A live remote project's tunnel, so discovery can read the remote `.claude`.
#[derive(serde::Deserialize)]
pub struct RemoteArg {
    pub ws_url: String,
    pub token: String,
}

/// Discover the MCP servers + skills a user already configured in Claude Code,
/// so they can be imported with one click (skills are picked up automatically).
/// Reads through an executor: local disk, or — when `remote` is given — the
/// remote host's `.claude` over the exec-server tunnel.
#[tauri::command]
pub async fn claude_discover(
    cwd: String,
    remote: Option<RemoteArg>,
) -> Result<ClaudeDiscovery, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec: Box<dyn provider_local::Executor> = match remote {
        Some(r) => Box::new(provider_local::RemoteExecutor::connect(&r.ws_url, &r.token).await?),
        None => Box::new(provider_local::LocalExecutor),
    };
    Ok(ClaudeDiscovery {
        mcp: provider_local::discover_mcp_servers(exec.as_ref(), &root).await,
        skills: provider_local::discover_skills(exec.as_ref(), &root).await,
    })
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

/// One per-fact memory file, flattened for the UI.
#[derive(serde::Serialize)]
pub struct MemoryFactView {
    pub file: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub body: String,
}

/// Everything the memory viewer needs for one scope (project or global).
#[derive(serde::Serialize)]
pub struct MemoryOverview {
    /// Absolute path to the scope's `.clark/memory` directory.
    pub dir: String,
    /// Whether the scope holds any memory (an index or at least one fact).
    pub exists: bool,
    /// Contents of the always-loaded `MEMORY.md` index, if present.
    pub index: Option<String>,
    /// Per-fact memory files (newest first).
    pub facts: Vec<MemoryFactView>,
}

/// Read one scope's `.clark/memory` directory into a viewer overview. The
/// directory is always local here (the desktop machine), so `LocalExecutor`.
async fn memory_overview(mem_dir: &std::path::Path) -> MemoryOverview {
    use provider_local::LocalExecutor;
    let facts_raw = provider_local::load_facts(&LocalExecutor, mem_dir).await;
    let index = provider_local::load_index(&LocalExecutor, mem_dir).await;
    let exists = index.is_some() || !facts_raw.is_empty();
    let facts = facts_raw
        .into_iter()
        .map(|f| MemoryFactView {
            file: f.header.file,
            name: f.header.name,
            description: f.header.description,
            kind: f.header.kind.map(|k| k.label().to_string()),
            body: f.body,
        })
        .collect();
    MemoryOverview {
        dir: mem_dir.to_string_lossy().to_string(),
        exists,
        index,
        facts,
    }
}

/// List the project-scoped memory for `cwd` (`<cwd>/.clark/memory/`). Read-only.
#[tauri::command]
pub async fn local_list_memory(cwd: String) -> Result<MemoryOverview, String> {
    if cwd.trim().is_empty() {
        return Err("choose a project folder first".into());
    }
    let mem_dir = provider_local::memory_dir(std::path::Path::new(&cwd));
    Ok(memory_overview(&mem_dir).await)
}

/// List the user's global memory (`~/.clark/memory/`). Read-only.
#[tauri::command]
pub async fn local_list_global_memory() -> Result<MemoryOverview, String> {
    let Some(mem_dir) = provider_local::global_memory_dir() else {
        return Err("could not resolve your home directory".into());
    };
    Ok(memory_overview(&mem_dir).await)
}

/// List project-relative file paths under `cwd` for the `@`-mention picker.
/// Read-only; skips ignored directories. Runs the walk off the UI thread.
#[tauri::command]
pub async fn local_list_files(cwd: String) -> Result<Vec<String>, String> {
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = std::path::PathBuf::from(cwd);
    tokio::task::spawn_blocking(move || provider_local::list_project_files(&root))
        .await
        .map_err(|e| format!("list files failed: {e}"))
}

/// Open a file (or folder) with the OS default handler — for a source file on a
/// dev machine that's typically the user's editor. `reveal` shows it in the file
/// manager instead of opening it. Never executes the file directly.
#[tauri::command]
pub fn open_path(path: String, reveal: bool) -> Result<(), String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("empty path".into());
    }
    let mut cmd = open_command(p, reveal);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    let mut c = std::process::Command::new("open");
    if reveal {
        c.arg("-R");
    }
    c.arg(path);
    c
}

#[cfg(target_os = "windows")]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    if reveal {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{path}"));
        c
    } else {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", path]);
        c
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command(path: &str, reveal: bool) -> std::process::Command {
    // No portable "reveal" on Linux — open the containing folder instead.
    let target = if reveal {
        std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    };
    let mut c = std::process::Command::new("xdg-open");
    c.arg(target);
    c
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
