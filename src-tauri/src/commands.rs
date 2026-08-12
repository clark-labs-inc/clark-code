//! Tauri command surface — the IPC boundary the web UI calls via `invoke`.
//! These mirror the `agent_core::Provider` trait and drive the live provider.

mod cloud;
mod cloud_authority;
mod cloud_conversations;
mod computer_use;
mod desktop_artifacts;
mod local;
mod product;
mod project;
mod prompt_admission;
mod provider_launch;
mod remote_worker;
mod session_close;
mod skills;
mod stream_batch;
pub use cloud::*;
pub use cloud_conversations::*;
pub use computer_use::*;
pub use desktop_artifacts::*;
pub use local::*;
pub use product::*;
pub(crate) use project::project_executor;
use prompt_admission::admit_and_append_prompt;
pub(crate) use provider_launch::ProviderLaunchRequest;
pub use remote_worker::*;
pub use session_close::*;
pub use skills::*;

use agent_core::provider::EventStream;
use agent_core::{
    apply, ClientResponse, CollaborationMode, ContentBlock, PendingUpload, PromptInput, Provider,
    ProviderConfig, RunId, Session, SessionId, Snapshot,
};
use agent_core::{AgentEvent, Role};
use futures::StreamExt;
use provider_acp::AcpProvider;
use provider_local::LocalAgentProvider;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::runtime_registry::{AccountKey, SessionKey};
use crate::ssh;
use crate::state::{ActiveRunGuard, HostSession};
use crate::trajectory::{CloudTrajectoryClient, CloudTrajectoryConfig};
use crate::{builtin_providers, AppState, ProviderInfo};

/// Synthetic run id used to attribute the user's own message in the timeline.
const USER_RUN: &str = "user";

fn batch_contains_terminal_run(events: &[AgentEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, AgentEvent::RunFinished { .. }))
}

fn record_conversation_diagnostics(conversation_id: &str, events: &[AgentEvent]) {
    for event in events {
        match event {
            AgentEvent::RunFinished { run, outcome } => {
                let failure_kind = outcome
                    .failure_kind
                    .as_ref()
                    .map(|kind| format!("{kind:?}"));
                if outcome.status == agent_core::RunStatus::Failed {
                    tracing::error!(
                        event = "conversation_run_finished",
                        conversation_id,
                        run_id = %run,
                        status = ?outcome.status,
                        failure_kind = failure_kind.as_deref().unwrap_or("none"),
                        stop_reason = outcome.stop_reason.as_deref().unwrap_or(""),
                        has_error = outcome.error.is_some(),
                        "conversation run failed"
                    );
                } else {
                    tracing::info!(
                        event = "conversation_run_finished",
                        conversation_id,
                        run_id = %run,
                        status = ?outcome.status,
                        stop_reason = outcome.stop_reason.as_deref().unwrap_or(""),
                        "conversation run finished"
                    );
                }
            }
            AgentEvent::Error { code, run, .. } => {
                tracing::error!(
                    event = "conversation_provider_error",
                    conversation_id,
                    run_id = run.as_ref().map(RunId::as_str).unwrap_or(""),
                    error_code = code,
                    "provider emitted a conversation error"
                );
            }
            _ => {}
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceipt {
    run_id: String,
}

/// Construct a provider instance by id.
async fn make_provider(
    id: &str,
    config: &ProviderConfig,
    app: &AppHandle,
    state: &AppState,
) -> Result<Box<dyn Provider>, String> {
    if let Some(provider) = state
        .product
        .make_provider(
            id,
            config,
            crate::product::ProductRequestContext { app, state },
        )
        .await?
    {
        return Ok(provider);
    }
    match id {
        "acp" => Ok(Box::new(AcpProvider::new())),
        "local" => Ok(Box::new(
            LocalAgentProvider::new()
                .with_skill_catalog_service(state.runtime_registry.current_skill_catalogs().await),
        )),
        other => Err(format!("unknown provider: {other}")),
    }
}

async fn prepare_provider_config(
    provider_id: &str,
    app: &AppHandle,
    mut config: ProviderConfig,
    state: &AppState,
) -> Result<(ProviderConfig, Option<AccountKey>), String> {
    if config.auth_token.is_some() {
        return Err("provider credentials must not cross the WebView boundary".into());
    }
    if let Some(prepared) = state
        .product
        .prepare_provider_config(
            provider_id,
            config.clone(),
            crate::product::ProductRequestContext { app, state },
        )
        .await?
    {
        let account = prepared.account_id.map(AccountKey::new).transpose()?;
        return Ok((prepared.config, account));
    }
    if provider_id != "local" {
        return Ok((config, None));
    }
    if config.extra.get("worker_execution_residency").is_some() {
        return Err(
            "worker execution residency is native-owned and must not cross the WebView boundary"
                .into(),
        );
    }
    if config.extra.get("scout_cartography").is_some() {
        return Err("this build has no Scout product integration".into());
    }
    if config.auth_token.is_none() {
        config.auth_token = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|credential| !credential.is_empty());
    }
    Ok((config, None))
}

pub(crate) async fn hydrate_mcp_servers(
    servers: &mut [provider_local::McpServerConfig],
    owner_scope: &str,
    state: &AppState,
) -> Result<(), String> {
    for server in servers {
        if server.env.values().any(|value| !value.is_empty()) {
            return Err(
                "MCP credential values must not cross the WebView configuration boundary".into(),
            );
        }
        let names = server.env.keys().cloned().collect::<Vec<_>>();
        if names.is_empty() {
            server.credential_ref = None;
            continue;
        }
        let credential_ref = server
            .credential_ref
            .as_deref()
            .ok_or("MCP credential reference is missing")?;
        server.env = state
            .credentials
            .mcp_environment(owner_scope, credential_ref, &names)
            .await?;
        server.credential_ref = None;
    }
    Ok(())
}

/// Persist and project the remainder of one provider-owned run stream. Prompt
/// and explicit compaction share this boundary so both get identical
/// write-ahead durability, stale-session rejection, and snapshot emission.
fn spawn_provider_stream(
    app: AppHandle,
    state: AppState,
    entry: Arc<Mutex<HostSession>>,
    session_key: SessionKey,
    stream: EventStream,
    run_guard: ActiveRunGuard,
) {
    tokio::spawn(async move {
        let _run_guard = run_guard;
        let mut stream = stream;
        while let Some(events) = stream_batch::next_event_batch(&mut stream).await {
            let _account_lifecycle = state.account_lifecycle.read().await;
            record_conversation_diagnostics(session_key.as_str(), &events);
            let specialist_projections = events
                .iter()
                .filter_map(|event| match event {
                    AgentEvent::Trace {
                        source, payload, ..
                    } if source == "product_projection" => Some(payload.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            // A forced close owns the same gate, so a late cancellation event
            // cannot reopen the snapshot after its terminal transition.
            let projection_gate = entry.lock().await.projection_gate.clone();
            let _projection = projection_gate.lock().await;
            // Stop if this session was closed or superseded by a reopen: the
            // captured provider must never clobber a newer session with the
            // same public conversation id.
            let still_current = state
                .runtime_registry
                .current_session_entry(&session_key)
                .await
                .is_some_and(|live| Arc::ptr_eq(&live, &entry));
            if !still_current {
                break;
            }
            let (trajectory, closing) = {
                let session = entry.lock().await;
                (session.trajectory.clone(), session.closing)
            };
            if closing {
                break;
            }
            let Some(trajectory) = trajectory else {
                break;
            };
            let checkpoint = match trajectory.append(&events).await {
                Ok(checkpoint) => checkpoint,
                Err(error) => {
                    tracing::error!(%error, "local trajectory outbox append failed; interrupting projection");
                    let _ = app.emit(
                        "cloud-sync-warning",
                        "Clark Code could not safely save the next part of this run, so it stopped at the last saved point.",
                    );
                    break;
                }
            };
            let snapshot = {
                let mut session = entry.lock().await;
                for event in &events {
                    apply(&mut session.snapshot, event);
                }
                session.snapshot.history_checkpoint = Some(checkpoint);
                session.snapshot.clone()
            };
            let _ = app.emit("snapshot", &snapshot);
            for payload in specialist_projections {
                match state
                    .product
                    .publish_projection(
                        &payload,
                        crate::product::ProductRequestContext {
                            app: &app,
                            state: &state,
                        },
                    )
                    .await
                {
                    Ok(Some(receipt)) => {
                        let _ = app.emit("specialist-projection-published", receipt);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "specialist overview publication failed");
                        let _ = app.emit(
                            "cloud-sync-warning",
                            format!(
                                "The specialist run is retained locally, but its overview could not be published: {error}"
                            ),
                        );
                    }
                }
            }
            // RunFinished is the provider contract's terminal boundary. Do not
            // keep the native update guard alive waiting for a provider stream
            // that failed to close after it already told the UI the run settled.
            if batch_contains_terminal_run(&events) {
                break;
            }
        }
    });
}

#[tauri::command]
pub fn provider_list(state: State<'_, AppState>) -> Vec<ProviderInfo> {
    state.product.providers(builtin_providers())
}

/// Files changed since a session baseline checkpoint (the Changes panel).
/// Read-only; runs git against a throwaway index off the UI thread.
#[tauri::command]
pub async fn changes_summary(
    cwd: String,
    base: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::ChangedFile>, String> {
    let exec = project_executor(remote, state.inner()).await?;
    provider_local::changes_summary(exec.as_ref(), std::path::Path::new(&cwd), &base).await
}

/// Unified diff of one file against the session baseline.
#[tauri::command]
pub async fn changes_diff(
    cwd: String,
    base: String,
    path: String,
    previous_path: Option<String>,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let exec = project_executor(remote, state.inner()).await?;
    provider_local::changes_diff(
        exec.as_ref(),
        std::path::Path::new(&cwd),
        &base,
        &path,
        previous_path.as_deref(),
    )
    .await
}

/// Restore one file to its baseline state (worktree only; created files are
/// removed). The user confirms in the panel before this fires.
#[tauri::command]
pub async fn changes_revert(
    cwd: String,
    base: String,
    path: String,
    previous_path: Option<String>,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let exec = project_executor(remote, state.inner()).await?;
    provider_local::changes_revert(
        exec.as_ref(),
        std::path::Path::new(&cwd),
        &base,
        &path,
        previous_path.as_deref(),
    )
    .await
}

/// Drop Clark Code's retention refs for checkpoints owned by a conversation that
/// the user permanently deleted.
#[tauri::command]
pub async fn changes_release_checkpoints(
    cwd: String,
    checkpoints: Vec<String>,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let exec = project_executor(remote, state.inner()).await?;
    provider_local::release_checkpoints(exec.as_ref(), std::path::Path::new(&cwd), &checkpoints)
        .await
}

/// Re-run `connect` on the EXISTING provider instance — unlike opening a new
/// provider, this keeps the live session (the model-visible
/// transcript lives in the provider), so the composer's model / reasoning-effort
/// picker can swap the LLM mid-conversation and the next turn continues with
/// full context on the new model.
#[tauri::command]
pub async fn provider_reconfigure(
    app: AppHandle,
    session_id: String,
    config: ProviderLaunchRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(session = %session_id, "reconfiguring live provider");
    let _account_lifecycle = state.account_lifecycle.read().await;
    let session_key = SessionKey::parse(session_id)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    let config = config.into_provider_config("local")?;
    let (config, account) = prepare_provider_config("local", &app, config, state.inner()).await?;
    if s.account.as_ref() != account.as_ref() {
        return Err("session belongs to a different desktop account".into());
    }
    s.provider.connect(config).await.map_err(|e| e.to_string())
}

/// Read-only "test connection": reach `host` and report its architecture + home,
/// without deploying or tunneling. Backs the SSH-host settings test button.
#[tauri::command]
pub async fn ssh_probe(host: String) -> Result<ssh::Probe, String> {
    tracing::info!(%host, "ssh_probe");
    ssh::probe(&host).await
}

/// Browse folders on an SSH host without deploying or starting an agent.
#[tauri::command]
pub async fn ssh_list_directories(
    host: String,
    path: Option<String>,
) -> Result<ssh::RemoteDirectoryListing, String> {
    tracing::info!(%host, path = path.as_deref().unwrap_or("$HOME"), "ssh_list_directories");
    ssh::list_directories(&host, path.as_deref()).await
}

/// An opaque native remote-worker capability, so discovery reads setup on the
/// worker rather than accidentally consulting the desktop filesystem.
#[derive(serde::Deserialize)]
pub struct RemoteArg {
    pub id: String,
}

/// Current branch and linked-worktree identity for the checkout shown above
/// the composer. A non-Git folder is a normal `None`, not an error.
#[tauri::command]
pub async fn project_context(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Option<crate::project_context::ProjectContext>, String> {
    let executor = project_executor(remote, state.inner()).await?;
    crate::project_context::inspect_project_context(
        executor.as_ref(),
        std::path::Path::new(cwd.trim()),
    )
    .await
}

/// Detect compatible MCP servers, skills, and instructions from other coding
/// agents. Discovery is read-only; the UI chooses which missing MCP servers to
/// add while skills and instructions remain sourced in place.
#[tauri::command]
pub async fn external_agent_discover(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::AgentMigrationDiscovery>, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote, state.inner()).await?;
    Ok(provider_local::discover_agent_setups(exec.as_ref(), &root).await)
}

/// List custom user-authored slash commands (`.claude/commands/*.md`,
/// project + personal) for the composer's `/` picker. Frontend-only concern
/// (unlike skills, which fold into the system prompt) — queried fresh on
/// `cwd` change rather than cached in session state.
#[tauri::command]
pub async fn list_commands(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::CustomCommand>, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote, state.inner()).await?;
    Ok(provider_local::discover_commands(exec.as_ref(), &root).await)
}

/// Insert a bound session into the pool (replacing any prior entry with the
/// same id — reopening a conversation supersedes its old, settled session) and
/// announce its clean snapshot. The client restores the persisted transcript;
/// starting clean means new turns append correctly.
async fn register_session(
    app: &AppHandle,
    state: &AppState,
    provider: Box<dyn Provider>,
    session: Session,
    account: Option<AccountKey>,
) -> Result<Value, String> {
    let mut snapshot = Snapshot::new();
    snapshot.session = Some(session.id.clone());
    let entry = HostSession {
        account: account.clone(),
        provider,
        session: session.clone(),
        snapshot: snapshot.clone(),
        trajectory: None,
        projection_gate: Arc::new(Mutex::new(())),
        closing: false,
    };
    let session_key = SessionKey::from_session(&session.id)?;
    let replaced = state
        .runtime_registry
        .bind_session(account, session_key, Arc::new(Mutex::new(entry)))
        .await?;
    // Edit-and-resend intentionally rebinds the same conversation id to a
    // provider resumed from an earlier transcript prefix. Close the displaced
    // provider after the map swap so its background work and resources cannot
    // leak, while its stream task sees that it is no longer current.
    if let Some(replaced) = replaced {
        let mut replaced = replaced.lock().await;
        let replaced_id = replaced.session.id.clone();
        if let Err(error) = replaced.provider.close_session(&replaced_id).await {
            tracing::warn!(%error, session = %replaced_id, "superseded provider close failed");
        }
    }
    let _ = app.emit("snapshot", &snapshot);
    serde_json::to_value(&session).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_configure_cloud(
    app: AppHandle,
    session_id: String,
    config: CloudTrajectoryConfig,
    base_snapshot: Value,
    base_rev: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session_key = SessionKey::parse(session_id.clone())?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let access = cloud_authority::current_account_access(state.inner()).await?;
    let outbox_path = crate::trajectory::outbox_path(&app)?;
    let owner_scope = access.owner_scope.clone();
    let account = AccountKey::new(owner_scope.clone())?;
    let trajectory = CloudTrajectoryClient::new(
        session_id,
        config,
        owner_scope.clone(),
        state.inner().clone(),
        app.clone(),
        outbox_path,
    )?;
    let base_snapshot = crate::trajectory::normalize_snapshot_value(base_snapshot);
    let base_snapshot: Snapshot = serde_json::from_value(base_snapshot)
        .map_err(|error| format!("decode session history snapshot: {error}"))?;
    trajectory.initialize(&base_snapshot, base_rev).await?;
    trajectory
        .append(&[AgentEvent::Trace {
            run: None,
            source: "desktop_session".into(),
            payload: serde_json::json!({"type": "session_configured"}),
        }])
        .await?;
    let still_current = state
        .runtime_registry
        .cloud_account()
        .await
        .is_some_and(|current| current.account.as_str() == owner_scope);
    if !still_current {
        return Err("Clark Code account changed while configuring the session".into());
    }
    let mut live = entry.lock().await;
    if live.account.as_ref().is_some_and(|bound| bound != &account) {
        return Err("session already belongs to a different desktop account".into());
    }
    live.account = Some(account);
    live.trajectory = Some(trajectory);
    Ok(())
}

/// Prevent new provider runs from starting and return the exact native count
/// still draining. The frontend polls this after its queued follow-ups settle;
/// installation begins only when it reaches zero.
#[tauri::command]
pub fn update_begin_drain(state: State<'_, AppState>) -> usize {
    state.begin_update_drain()
}

/// Release a failed/abandoned update drain so coding can continue normally.
#[tauri::command]
pub fn update_cancel_drain(state: State<'_, AppState>) {
    state.cancel_update_drain();
}

/// Inject a user message into the session's ACTIVE run (mid-run steering) —
/// it lands between tool batches instead of waiting for the run to finish.
/// Fails when the provider has no live run to steer; the frontend falls back
/// to its queued-message flow. On success the message is echoed into the
/// snapshot (providers don't re-emit steered input) and appended durably.
#[tauri::command]
pub async fn steer(
    app: AppHandle,
    session_id: String,
    blocks: Vec<ContentBlock>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;

    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("product cloud trajectory is not configured for this session")?;
    let durable = blocks
        .iter()
        .cloned()
        .map(|delta| AgentEvent::MessageChunk {
            run: RunId::new(USER_RUN),
            role: Role::User,
            delta,
        })
        .collect::<Vec<_>>();
    // Ask the provider first so a rejected steer is not journaled as accepted;
    // once accepted, commit it locally before rendering it.
    {
        let mut s = entry.lock().await;
        s.provider
            .steer(
                &sid,
                PromptInput {
                    blocks: blocks.clone(),
                    attachments: Vec::new(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let checkpoint = trajectory.append(&durable).await?;
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
        s.snapshot.history_checkpoint = Some(checkpoint);
        let _ = app.emit("snapshot", &s.snapshot);
    }
    Ok(())
}

#[tauri::command]
pub async fn prompt(
    app: AppHandle,
    session_id: String,
    blocks: Vec<ContentBlock>,
    attachments: Vec<PendingUpload>,
    state: State<'_, AppState>,
) -> Result<PromptReceipt, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let run_guard = state.try_start_run().ok_or(
        "local agent is finishing active work before an update; wait for the relaunch to send another message",
    )?;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;

    tracing::info!(
        event = "conversation_prompt_received",
        conversation_id = %sid,
        block_count = blocks.len(),
        attachment_count = attachments.len(),
        "conversation prompt received"
    );

    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("product cloud trajectory is not configured for this session")?;
    // The visible user turn is the text PLUS an echo of each attachment
    // (image thumbnail / file chip) — without it the timeline shows only the
    // text and the files the user attached seem to vanish on send.
    let echo_blocks: Vec<ContentBlock> = blocks
        .iter()
        .cloned()
        .chain(attachments.iter().map(PendingUpload::echo_block))
        .collect();
    let mut durable_prompt = vec![AgentEvent::Trace {
        run: None,
        source: "desktop_prompt".into(),
        payload: serde_json::json!({
            "blocks": blocks.clone(),
            "attachments": attachments.clone(),
        }),
    }];
    durable_prompt.extend(
        echo_blocks
            .iter()
            .cloned()
            .map(|delta| AgentEvent::MessageChunk {
                run: RunId::new(USER_RUN),
                role: Role::User,
                delta,
            }),
    );
    let prompt_input = PromptInput {
        blocks: blocks.clone(),
        attachments: attachments.clone(),
    };
    // Keep admission and the append-only journal write in one tested host
    // protocol. A rejected command returns before the journal can be touched.
    let prompt_checkpoint = {
        let s = entry.lock().await;
        admit_and_append_prompt(
            s.provider.as_ref(),
            &sid,
            &prompt_input,
            &trajectory,
            &durable_prompt,
        )
        .await?
    };

    // Show the user's message immediately (providers don't reliably echo it),
    // then lock the session to obtain the run's event stream and release.
    let provider_prompt = {
        let mut s = entry.lock().await;
        for block in &echo_blocks {
            apply(
                &mut s.snapshot,
                &AgentEvent::MessageChunk {
                    run: RunId::new(USER_RUN),
                    role: Role::User,
                    delta: block.clone(),
                },
            );
        }
        // Submission is in flight but the provider has not allocated a run yet.
        // Attachment upload / connect handshake can take seconds, so flag a
        // transient "starting" state on the snapshot before awaiting the
        // provider — otherwise the timeline sits static right after the user's
        // message appears. The first `RunStarted` (applied below) clears it via
        // the reducer; a rejection below clears it directly.
        s.snapshot.starting = true;
        s.snapshot.history_checkpoint = Some(prompt_checkpoint);
        let _ = app.emit("snapshot", &s.snapshot);

        s.provider
            .prompt(
                &sid,
                PromptInput {
                    blocks,
                    attachments,
                },
            )
            .await
    };
    let mut stream = match provider_prompt {
        Ok(stream) => stream,
        Err(error) => {
            // The prompt never produced a run; retire the transient starting
            // state so the activity row does not stay animated under the error.
            let mut s = entry.lock().await;
            s.snapshot.starting = false;
            let _ = app.emit("snapshot", &s.snapshot);
            tracing::error!(
                event = "conversation_prompt_rejected",
                conversation_id = %sid,
                "provider rejected the conversation prompt before allocating a run"
            );
            return Err(error.to_string());
        }
    };

    // Submission is not complete until the provider has allocated the run.
    // Persist and project that first lifecycle fact before returning its ID so
    // mobile command receipts and the trajectory can share one identity.
    let first = stream
        .next()
        .await
        .ok_or("local agent prompt ended before it allocated a run")?;
    let run_id = match &first {
        AgentEvent::RunStarted { run } => run.as_str().to_string(),
        _ => return Err("local agent prompt did not begin with a run identity".into()),
    };
    tracing::info!(
        event = "conversation_run_allocated",
        conversation_id = %sid,
        run_id = %run_id,
        "provider allocated a conversation run"
    );
    let checkpoint = trajectory.append(std::slice::from_ref(&first)).await?;
    let snapshot = {
        let mut session = entry.lock().await;
        apply(&mut session.snapshot, &first);
        session.snapshot.history_checkpoint = Some(checkpoint);
        session.snapshot.clone()
    };
    let _ = app.emit("snapshot", &snapshot);

    // Fold events into this session's snapshot and push each update to the
    // webview (tagged by `snapshot.session`, so the UI routes it correctly).
    spawn_provider_stream(
        app,
        state.inner().clone(),
        entry,
        session_key,
        stream,
        run_guard,
    );
    Ok(PromptReceipt { run_id })
}

/// Explicit local agent context compaction. This is a standalone provider run,
/// not a user prompt: `/compact` never enters the model transcript as a user
/// instruction. The first lifecycle event is projected before returning so the
/// composer cannot race a new prompt into the history replacement.
#[tauri::command]
pub async fn compact(
    app: AppHandle,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let run_guard = state.try_start_run().ok_or(
        "local agent is finishing active work before an update; wait for the relaunch to compact context",
    )?;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("product cloud trajectory is not configured for this session")?;
    let mut stream = {
        let mut session = entry.lock().await;
        session
            .provider
            .compact(&sid)
            .await
            .map_err(|error| error.to_string())?
    };

    let first = stream
        .next()
        .await
        .ok_or("context compaction ended before it started")?;
    let checkpoint = trajectory.append(std::slice::from_ref(&first)).await?;
    let snapshot = {
        let mut session = entry.lock().await;
        apply(&mut session.snapshot, &first);
        session.snapshot.history_checkpoint = Some(checkpoint);
        session.snapshot.clone()
    };
    let _ = app.emit("snapshot", &snapshot);

    spawn_provider_stream(
        app,
        state.inner().clone(),
        entry,
        session_key,
        stream,
        run_guard,
    );
    Ok(())
}

#[tauri::command]
pub async fn cancel(
    session_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .cancel(&sid, &RunId::new(run_id))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn respond(
    session_id: String,
    response: ClientResponse,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(
        event = "conversation_response_received",
        conversation_id = %session_id,
        response_kind = match &response {
            ClientResponse::Permission { .. } => "permission",
            ClientResponse::PlanDecision { .. } => "plan_decision",
        },
        "conversation response received"
    );
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(&session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .respond(&sid, response)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        event = "conversation_response_applied",
        conversation_id = %session_id,
        "conversation response applied"
    );
    Ok(())
}

#[tauri::command]
pub async fn set_mode(
    session_id: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .set_mode(&sid, mode)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_collaboration_mode(
    session_id: String,
    mode: CollaborationMode,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut session = entry.lock().await;
    session
        .provider
        .set_collaboration_mode(&sid, mode)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_output_style(
    session_id: String,
    style: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let sid = SessionId::new(session_id);
    let session_key = SessionKey::from_session(&sid)?;
    let entry = state
        .runtime_registry
        .current_session_entry(&session_key)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .set_output_style(&sid, style)
        .await
        .map_err(|e| e.to_string())
}

/// Real-backend coverage for the Tauri commands that have no `State<AppState>`
/// dependency (`list_commands`, `changes_*`) — the
/// exact functions the webview's `invoke()` calls, exercised directly against a
/// real temp git repo and real files. No mocking: real `git`, real filesystem,
/// real `provider_local::` logic. This exists because GUI automation of the
/// actual Tauri window (screenshots, synthetic clicks) is blocked in this
/// environment by macOS TCC permissions (Accessibility "assistive access" +
/// Screen Recording) that require a one-time manual grant the session
/// couldn't perform — see the conversation this landed in for the full story.
#[cfg(test)]
mod real_backend_tests;

pub(crate) mod session_open;

#[cfg(test)]
mod tests {
    use super::batch_contains_terminal_run;
    use agent_core::{AgentEvent, RunId, RunOutcome, RunStatus};

    #[test]
    fn terminal_run_event_ends_the_native_drain_boundary() {
        let started = [AgentEvent::RunStarted {
            run: RunId::new("run-1"),
        }];
        assert!(!batch_contains_terminal_run(&started));

        let finished = [AgentEvent::RunFinished {
            run: RunId::new("run-1"),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        }];
        assert!(batch_contains_terminal_run(&finished));
    }
}
