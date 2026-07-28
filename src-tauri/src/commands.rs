//! Tauri command surface — the IPC boundary the web UI calls via `invoke`.
//! These mirror the `agent_core::Provider` trait and drive the live provider.

mod cloud;
mod cloud_authority;
mod cloud_conversations;
mod computer_use;
mod local;
mod project;
mod session_close;
mod skills;
pub use cloud::*;
pub(crate) use cloud_authority::{clark_http_client, clark_rest_base, jwt_subject};
pub use cloud_conversations::*;
pub use computer_use::*;
pub use local::*;
use project::project_executor;
pub use session_close::*;
pub use skills::*;

use agent_core::provider::EventStream;
use agent_core::{
    apply, ClientResponse, CollaborationMode, ContentBlock, PendingUpload, PromptInput, Provider,
    ProviderConfig, RunId, Session, SessionId, SessionOptions, Snapshot,
};
use agent_core::{AgentEvent, Role};
use futures::StreamExt;
use provider_acp::AcpProvider;
use provider_clark::ClarkProvider;
use provider_local::LocalAgentProvider;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::ssh::{self, RemoteSpec};
use crate::state::{ActiveRunGuard, HostSession};
use crate::trajectory::{CloudTrajectoryClient, CloudTrajectoryConfig};
use crate::{builtin_providers, AppState, ProviderInfo};

/// Synthetic run id used to attribute the user's own message in the timeline.
const USER_RUN: &str = "user";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceipt {
    run_id: String,
}

/// Construct a provider instance by id.
fn make_provider(id: &str, state: &AppState) -> Result<Box<dyn Provider>, String> {
    match id {
        "acp" => Ok(Box::new(AcpProvider::new())),
        "clark" => Ok(Box::new(ClarkProvider::new())),
        "local" => Ok(Box::new(
            LocalAgentProvider::new().with_skill_catalog_service(state.skill_catalogs.clone()),
        )),
        other => Err(format!("unknown provider: {other}")),
    }
}

/// Persist and project the remainder of one provider-owned run stream. Prompt
/// and explicit compaction share this boundary so both get identical
/// write-ahead durability, stale-session rejection, and snapshot emission.
fn spawn_provider_stream(
    app: AppHandle,
    state: AppState,
    entry: Arc<Mutex<HostSession>>,
    session_key: String,
    stream: EventStream,
    run_guard: ActiveRunGuard,
) {
    tokio::spawn(async move {
        let _run_guard = run_guard;
        let mut batches = stream.ready_chunks(64);
        while let Some(events) = batches.next().await {
            // A forced close owns the same gate, so a late cancellation event
            // cannot reopen the snapshot after its terminal transition.
            let projection_gate = entry.lock().await.projection_gate.clone();
            let _projection = projection_gate.lock().await;
            // Stop if this session was closed or superseded by a reopen: the
            // captured provider must never clobber a newer session with the
            // same public conversation id.
            let still_current = state
                .session_entry(&session_key)
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
                        "Clark could not safely save the next part of this run, so it stopped at the last saved point.",
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
        }
    });
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
    let mut provider = make_provider(&provider_id, state.inner())?;
    provider.connect(config).await.map_err(|e| e.to_string())?;
    // Parked until `session_new`/`session_load` binds it to a session. Each
    // session gets its own provider instance, so any number can stream at once.
    state.pending_provider.lock().await.replace(provider);
    Ok(())
}

/// Files changed since a session baseline checkpoint (the Changes panel).
/// Read-only; runs git against a throwaway index off the UI thread.
#[tauri::command]
pub async fn changes_summary(
    cwd: String,
    base: String,
    remote: Option<RemoteArg>,
) -> Result<Vec<provider_local::ChangedFile>, String> {
    let exec = project_executor(remote).await?;
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
) -> Result<String, String> {
    let exec = project_executor(remote).await?;
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
) -> Result<(), String> {
    let exec = project_executor(remote).await?;
    provider_local::changes_revert(
        exec.as_ref(),
        std::path::Path::new(&cwd),
        &base,
        &path,
        previous_path.as_deref(),
    )
    .await
}

/// Drop Clark's retention refs for checkpoints owned by a conversation that
/// the user permanently deleted.
#[tauri::command]
pub async fn changes_release_checkpoints(
    cwd: String,
    checkpoints: Vec<String>,
    remote: Option<RemoteArg>,
) -> Result<(), String> {
    let exec = project_executor(remote).await?;
    provider_local::release_checkpoints(exec.as_ref(), std::path::Path::new(&cwd), &checkpoints)
        .await
}

/// Re-run `connect` on the EXISTING provider instance — unlike
/// [`provider_connect`], this keeps the live session (the model-visible
/// transcript lives in the provider), so the composer's model / reasoning-effort
/// picker can swap the LLM mid-conversation and the next turn continues with
/// full context on the new model.
#[tauri::command]
pub async fn provider_reconfigure(
    session_id: String,
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(session = %session_id, "reconfiguring live provider");
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider.connect(config).await.map_err(|e| e.to_string())
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

/// A live remote project's tunnel, so discovery reads setup on the executor's
/// machine rather than accidentally consulting the desktop filesystem.
#[derive(serde::Deserialize)]
pub struct RemoteArg {
    pub ws_url: String,
    pub token: String,
}

/// Current branch and linked-worktree identity for the checkout shown above
/// the composer. A non-Git folder is a normal `None`, not an error.
#[tauri::command]
pub async fn project_context(
    cwd: String,
    remote: Option<RemoteArg>,
) -> Result<Option<crate::project_context::ProjectContext>, String> {
    let executor = project_executor(remote).await?;
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
) -> Result<Vec<provider_local::AgentMigrationDiscovery>, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote).await?;
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
) -> Result<Vec<provider_local::CustomCommand>, String> {
    let root = std::path::PathBuf::from(cwd);
    let exec = project_executor(remote).await?;
    Ok(provider_local::discover_commands(exec.as_ref(), &root).await)
}

/// Bind the pending (just-connected) provider to a fresh session and register
/// it in the live-session pool. `bind_id` — when the frontend reopens an
/// existing conversation on a provider that can't resume — is the conversation
/// id the frontend will address this session with; the session is keyed (and
/// its snapshot tagged) by it so events route to the right conversation.
#[tauri::command]
pub async fn session_new(
    provider_id: String,
    options: SessionOptions,
    bind_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    tracing::info!(provider = %provider_id, bind = bind_id.as_deref().unwrap_or(""), "session_new");
    let mut provider = state
        .pending_provider
        .lock()
        .await
        .take()
        .ok_or("connect a provider first")?;
    let mut session = provider
        .new_session(options)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(bind) = bind_id {
        // The provider ignores the wire session id (it is single-session); the
        // pool key and everything the frontend sees use the conversation id.
        session.id = SessionId::new(bind);
    }
    register_session(&app, &state, provider, session).await
}

#[tauri::command]
pub async fn session_load(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    tracing::info!(session = %id, "session_load");
    let mut provider = state
        .pending_provider
        .lock()
        .await
        .take()
        .ok_or("connect a provider first")?;
    let session = provider
        .load_session(SessionId::new(id))
        .await
        .map_err(|e| e.to_string())?;
    register_session(&app, &state, provider, session).await
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
) -> Result<Value, String> {
    let mut snapshot = Snapshot::new();
    snapshot.session = Some(session.id.clone());
    let entry = HostSession {
        provider,
        session: session.clone(),
        snapshot: snapshot.clone(),
        trajectory: None,
        projection_gate: Arc::new(Mutex::new(())),
        closing: false,
    };
    let replaced = state
        .sessions
        .lock()
        .await
        .insert(session.id.to_string(), Arc::new(Mutex::new(entry)));
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
    mut config: CloudTrajectoryConfig,
    base_snapshot: Snapshot,
    base_rev: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let access =
        cloud_authority::require_cloud_access(state.inner(), &config.endpoint, &config.token)
            .await?;
    config.endpoint = access.rest_base;
    let outbox_path = crate::trajectory::outbox_path(&app)?;
    let trajectory = CloudTrajectoryClient::new(
        session_id,
        config,
        access.owner_scope,
        state.cloud_token.clone(),
        app.clone(),
        outbox_path,
    )?;
    trajectory.initialize(&base_snapshot, base_rev).await?;
    trajectory
        .append(&[AgentEvent::Trace {
            run: None,
            source: "clark_desktop_session".into(),
            payload: serde_json::json!({"type": "session_configured"}),
        }])
        .await?;
    entry.lock().await.trajectory = Some(trajectory);
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
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let sid = SessionId::new(session_id);

    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("Clark cloud trajectory is not configured for this session")?;
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
    let run_guard = state.try_start_run().ok_or(
        "Clark Code is finishing active work before an update; wait for the relaunch to send another message",
    )?;
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let sid = SessionId::new(session_id);

    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("Clark cloud trajectory is not configured for this session")?;
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
        source: "clark_desktop_prompt".into(),
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
    let prompt_checkpoint = trajectory.append(&durable_prompt).await?;

    // Show the user's message immediately (providers don't reliably echo it),
    // then lock the session to obtain the run's event stream and release.
    let mut stream = {
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
            .map_err(|e| e.to_string())?
    };

    // Submission is not complete until the provider has allocated the run.
    // Persist and project that first lifecycle fact before returning its ID so
    // mobile command receipts and the trajectory can share one identity.
    let first = stream
        .next()
        .await
        .ok_or("Clark Code prompt ended before it allocated a run")?;
    let run_id = match &first {
        AgentEvent::RunStarted { run } => run.as_str().to_string(),
        _ => return Err("Clark Code prompt did not begin with a run identity".into()),
    };
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
        sid.as_str().to_string(),
        stream,
        run_guard,
    );
    Ok(PromptReceipt { run_id })
}

/// Explicit Clark Code context compaction. This is a standalone provider run,
/// not a user prompt: `/compact` never enters the model transcript as a user
/// instruction. The first lifecycle event is projected before returning so the
/// composer cannot race a new prompt into the history replacement.
#[tauri::command]
pub async fn compact(
    app: AppHandle,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let run_guard = state.try_start_run().ok_or(
        "Clark Code is finishing active work before an update; wait for the relaunch to compact context",
    )?;
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let sid = SessionId::new(session_id);
    let trajectory = entry
        .lock()
        .await
        .trajectory
        .clone()
        .ok_or("Clark cloud trajectory is not configured for this session")?;
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
        sid.as_str().to_string(),
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
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
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
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
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
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .set_mode(&SessionId::new(session_id), mode)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_collaboration_mode(
    session_id: String,
    mode: CollaborationMode,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut session = entry.lock().await;
    session
        .provider
        .set_collaboration_mode(&SessionId::new(session_id), mode)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_output_style(
    session_id: String,
    style: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let entry = state
        .session_entry(&session_id)
        .await
        .ok_or("no such session")?;
    let mut s = entry.lock().await;
    s.provider
        .set_output_style(&SessionId::new(session_id), style)
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
