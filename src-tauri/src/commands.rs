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

/// Files changed since a session baseline checkpoint (the Changes panel).
/// Read-only; runs git against a throwaway index off the UI thread.
#[tauri::command]
pub async fn changes_summary(
    cwd: String,
    base: String,
) -> Result<Vec<provider_local::ChangedFile>, String> {
    tokio::task::spawn_blocking(move || {
        provider_local::changes_summary(std::path::Path::new(&cwd), &base)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Unified diff of one file against the session baseline.
#[tauri::command]
pub async fn changes_diff(cwd: String, base: String, path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        provider_local::changes_diff(std::path::Path::new(&cwd), &base, &path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Restore one file to its baseline state (worktree only; created files are
/// removed). The user confirms in the panel before this fires.
#[tauri::command]
pub async fn changes_revert(cwd: String, base: String, path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        provider_local::changes_revert(std::path::Path::new(&cwd), &base, &path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Re-run `connect` on the EXISTING provider instance — unlike
/// [`provider_connect`], this keeps the live session (the model-visible
/// transcript lives in the provider), so the composer's model / reasoning-effort
/// picker can swap the LLM mid-conversation and the next turn continues with
/// full context on the new model.
#[tauri::command]
pub async fn provider_reconfigure(
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!("reconfiguring live provider");
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    provider.connect(config).await.map_err(|e| e.to_string())
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

/// List custom user-authored slash commands (`.claude/commands/*.md`,
/// project + personal) for the composer's `/` picker. Frontend-only concern
/// (unlike skills, which fold into the system prompt) — queried fresh on
/// `cwd` change rather than cached in session state.
#[tauri::command]
pub async fn list_commands(
    cwd: String,
    remote: Option<RemoteArg>,
) -> Result<Vec<provider_local::CustomCommand>, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec: Box<dyn provider_local::Executor> = match remote {
        Some(r) => Box::new(provider_local::RemoteExecutor::connect(&r.ws_url, &r.token).await?),
        None => Box::new(provider_local::LocalExecutor),
    };
    Ok(provider_local::discover_commands(exec.as_ref(), &root).await)
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

#[tauri::command]
pub async fn set_mode(
    session_id: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    provider
        .set_mode(&SessionId::new(session_id), mode)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_output_style(
    session_id: String,
    style: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut s = state.session.lock().await;
    let provider = s.provider.as_mut().ok_or("connect a provider first")?;
    provider
        .set_output_style(&SessionId::new(session_id), style)
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

/// Read an agent-authored document (Markdown) so the UI can render it inline.
/// Confined to the app-managed workspace (`~/.clark/workspace`) — it never reads
/// arbitrary files — and capped so a pathological file can't be slurped whole.
#[tauri::command]
pub async fn read_doc_text(path: String) -> Result<String, String> {
    const MAX_DOC_BYTES: u64 = 4 * 1024 * 1024;
    let root = provider_local::workspace_root()
        .ok_or_else(|| "no workspace directory".to_string())?
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path is outside the document workspace".into());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_DOC_BYTES {
        return Err("document too large to preview".into());
    }
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&canon).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("read failed: {e}"))?
}

/// Read a locally-captured screenshot (or other small image) from the
/// app-managed workspace and return it as a `data:` URL for inline `<img>`
/// rendering. Confined to `~/.clark/workspace`, same root and containment
/// check as `read_doc_text`.
#[tauri::command]
pub async fn read_image_data_url(path: String) -> Result<String, String> {
    use base64::Engine as _;

    const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
    let root = provider_local::workspace_root()
        .ok_or_else(|| "no workspace directory".to_string())?
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path is outside the document workspace".into());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("image too large to preview".into());
    }
    let mime = match canon
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => return Err("not a supported image type".into()),
    };
    let bytes =
        tokio::task::spawn_blocking(move || std::fs::read(&canon).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("read failed: {e}"))??;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
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
    rev: i64,
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
            "rev": rev,
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

/// Real-backend coverage for the Tauri commands that have no `State<AppState>`
/// dependency (`list_commands`, `changes_*`, `clark_checkpoint_restore`) — the
/// exact functions the webview's `invoke()` calls, exercised directly against a
/// real temp git repo and real files. No mocking: real `git`, real filesystem,
/// real `provider_local::` logic. This exists because GUI automation of the
/// actual Tauri window (screenshots, synthetic clicks) is blocked in this
/// environment by macOS TCC permissions (Accessibility "assistive access" +
/// Screen Recording) that require a one-time manual grant the session
/// couldn't perform — see the conversation this landed in for the full story.
#[cfg(test)]
mod real_backend_tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git available");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    fn init_repo(dir: &std::path::Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "initial"]);
    }

    #[tokio::test]
    async fn list_commands_discovers_a_real_claude_commands_file() {
        let dir = tempfile::tempdir().unwrap();
        let cmd_dir = dir.path().join(".claude/commands");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        std::fs::write(
            cmd_dir.join("review.md"),
            "---\ndescription: Review the current diff.\n---\n\nReview the current diff for bugs.",
        )
        .unwrap();

        let found = list_commands(dir.path().to_string_lossy().to_string(), None)
            .await
            .expect("list_commands succeeds against a real directory");
        let review = found
            .iter()
            .find(|c| c.name == "review")
            .expect("the real .claude/commands/review.md was discovered");
        assert_eq!(review.description, "Review the current diff.");
        assert_eq!(review.body, "Review the current diff for bugs.");
    }

    #[tokio::test]
    async fn list_commands_is_empty_for_a_project_with_no_commands_dir() {
        let dir = tempfile::tempdir().unwrap();
        let found = list_commands(dir.path().to_string_lossy().to_string(), None)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn changes_summary_and_diff_see_a_real_edit_against_a_real_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());

        // A real checkpoint, via the exact function `engine.rs` calls at the
        // start of every turn.
        let base = provider_local::create_checkpoint(dir.path())
            .expect("real git repo checkpoints successfully");

        // A real, independent edit after the checkpoint.
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("new_file.rs"), "// new\n").unwrap();

        let cwd = dir.path().to_string_lossy().to_string();
        let summary = changes_summary(cwd.clone(), base.clone())
            .await
            .expect("changes_summary succeeds against a real checkpoint");
        assert!(summary
            .iter()
            .any(|f| f.path == "main.rs" && f.status == "modified"));
        assert!(summary
            .iter()
            .any(|f| f.path == "new_file.rs" && f.status == "added"));

        let diff = changes_diff(cwd.clone(), base.clone(), "main.rs".to_string())
            .await
            .expect("changes_diff succeeds");
        assert!(
            diff.contains("println"),
            "real diff should show the real edit: {diff}"
        );

        // Revert just the one file — the real filesystem should show the
        // original content again, and the new file should be untouched.
        changes_revert(cwd.clone(), base.clone(), "main.rs".to_string())
            .await
            .expect("changes_revert succeeds");
        let restored = std::fs::read_to_string(dir.path().join("main.rs")).unwrap();
        assert_eq!(restored, "fn main() {}\n");
        assert!(dir.path().join("new_file.rs").exists());
    }

    #[tokio::test]
    async fn clark_checkpoint_restore_reverts_the_whole_real_tree() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let base = provider_local::create_checkpoint(dir.path()).unwrap();

        std::fs::write(dir.path().join("main.rs"), "fn main() { /* changed */ }\n").unwrap();
        std::fs::write(
            dir.path().join("scratch.rs"),
            "// created after checkpoint\n",
        )
        .unwrap();

        let cwd = dir.path().to_string_lossy().to_string();
        clark_checkpoint_restore(cwd, base)
            .await
            .expect("clark_checkpoint_restore succeeds against a real checkpoint");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert!(
            !dir.path().join("scratch.rs").exists(),
            "a whole-tree restore should remove files created after the checkpoint"
        );
    }
}
