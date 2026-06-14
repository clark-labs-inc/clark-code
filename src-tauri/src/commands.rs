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
