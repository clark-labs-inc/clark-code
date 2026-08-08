use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use agent_core::provider::{ClientResponse, PromptInput, Provider, Session, SessionOptions};
use agent_core::{apply, AgentEvent, Error as AgentError, RunId, RunStatus, Snapshot};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use code_host::{HeadlessPlugin, PluginContext, PluginError, PluginManifest};

use crate::config::{ExecutionResidency, ProviderProfile};

const SESSION_OPEN: &str = "session.open";
const SESSION_PROMPT: &str = "session.prompt";
const SESSION_RESPOND: &str = "session.respond";
const SESSION_CLOSE: &str = "session.close";

struct CodingSession {
    project_id: String,
    provider: Box<dyn Provider>,
    session: Session,
    snapshot: Snapshot,
}

/// First-party coding plugin. Its only dependency on the host is the
/// provider trait, so the same plugin can be mounted in a Tauri process, a
/// local worker, or a remote GPU worker without copying the agent loop.
pub struct CodingPlugin {
    manifest: PluginManifest,
    profile: ProviderProfile,
    execution_residency: ExecutionResidency,
    sessions: Arc<Mutex<BTreeMap<String, Arc<Mutex<CodingSession>>>>>,
}

impl CodingPlugin {
    pub fn new(profile: ProviderProfile, execution_residency: ExecutionResidency) -> Self {
        Self {
            manifest: PluginManifest {
                id: "coding".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "Coding-agent provider sessions and bounded tool execution".into(),
                operations: BTreeSet::from([
                    SESSION_OPEN.into(),
                    SESSION_PROMPT.into(),
                    SESSION_RESPOND.into(),
                    SESSION_CLOSE.into(),
                ]),
                capabilities: BTreeSet::from([
                    "provider.local".into(),
                    "stream.events".into(),
                    "trajectory.snapshot".into(),
                    "permissions.allowlist".into(),
                ]),
            },
            profile,
            execution_residency,
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    async fn open(&self, context: PluginContext, input: Value) -> Result<Value, PluginError> {
        let project_id = context.project_id.ok_or_else(|| {
            PluginError::InvalidInput("session.open requires a registered project_id".into())
        })?;
        let project_root = context.project_root.ok_or_else(|| {
            PluginError::InvalidInput("session.open requires a registered project_id".into())
        })?;
        let mut request: OpenRequest = decode(input)?;
        if request
            .options
            .cwd
            .as_deref()
            .is_some_and(|cwd| std::path::Path::new(cwd) != project_root)
        {
            return Err(PluginError::InvalidInput(
                "session.open cwd does not match the registered project root".into(),
            ));
        }
        request.options.cwd = Some(project_root.to_string_lossy().into_owned());
        let id = request
            .session_id
            .unwrap_or_else(|| format!("session-{}", uuid::Uuid::new_v4().simple()));
        portable_session_id(&id)?;
        let mut provider = provider_local::LocalAgentProvider::new();
        provider
            .connect(self.profile.provider_config(self.execution_residency))
            .await
            .map_err(provider_error)?;
        let session = provider
            .new_session(request.options)
            .await
            .map_err(provider_error)?;
        let session_id = session.id.to_string();
        let mut sessions = self.sessions.lock().await;
        if sessions.contains_key(&id) {
            drop(sessions);
            let _ = provider.close_session(&session.id).await;
            return Err(PluginError::InvalidInput(format!(
                "session id already exists: {id}"
            )));
        }
        sessions.insert(
            id.clone(),
            Arc::new(Mutex::new(CodingSession {
                project_id: project_id.clone(),
                provider: Box::new(provider),
                session: session.clone(),
                snapshot: Snapshot {
                    session: Some(session.id.clone()),
                    ..Snapshot::default()
                },
            })),
        );
        Ok(json!({
            "session_id": id,
            "provider_session_id": session_id,
            "project_id": project_id,
            "capabilities": session.capabilities,
        }))
    }

    async fn prompt(&self, context: PluginContext, input: Value) -> Result<Value, PluginError> {
        let request: PromptRequest = decode(input)?;
        if request.input.blocks.is_empty() && request.input.attachments.is_empty() {
            return Err(PluginError::InvalidInput(
                "session.prompt requires content or an attachment".into(),
            ));
        }
        let handle = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| {
                PluginError::InvalidInput(format!("unknown session: {}", request.session_id))
            })?;
        let (session_id, mut stream) = {
            let mut state = handle.lock().await;
            if context.cancellation.is_cancelled() {
                return Err(PluginError::Cancelled);
            }
            if context.project_id.as_deref() != Some(state.project_id.as_str()) {
                return Err(PluginError::InvalidInput(
                    "session.prompt project_id does not match the session checkout".into(),
                ));
            }
            let session_id = state.session.id.clone();
            state
                .provider
                .validate_prompt(&session_id, &request.input)
                .await
                .map_err(provider_error)?;
            let stream = state
                .provider
                .prompt(&session_id, request.input)
                .await
                .map_err(provider_error)?;
            (session_id, stream)
        };
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(self.profile.turn_timeout_seconds);
        let mut outcome = None;
        let mut run_id: Option<RunId> = None;
        loop {
            let next = tokio::select! {
                _ = context.cancellation.cancelled() => {
                    if let Some(run) = run_id.as_ref() {
                        let _ = handle.lock().await.provider.cancel(&session_id, run).await;
                    }
                    return Err(PluginError::Cancelled);
                }
                next = tokio::time::timeout_at(deadline, stream.next()) => {
                    next.map_err(|_| PluginError::Failed(format!(
                        "coding turn exceeded {} seconds",
                        self.profile.turn_timeout_seconds
                    )))?
                }
            };
            let Some(event) = next else {
                break;
            };
            if let AgentEvent::RunStarted { run } = &event {
                run_id = Some(run.clone());
            }
            if let AgentEvent::RunFinished {
                outcome: terminal, ..
            } = &event
            {
                outcome = Some(terminal.clone());
            }
            apply(&mut handle.lock().await.snapshot, &event);
            context
                .progress
                .emit(
                    "agent_event",
                    serde_json::to_value(&event)
                        .map_err(|error| PluginError::Failed(error.to_string()))?,
                )
                .await?;
            if outcome.as_ref().is_some_and(|terminal| {
                matches!(
                    terminal.status,
                    RunStatus::Done | RunStatus::Failed | RunStatus::Cancelled
                )
            }) {
                break;
            }
        }
        if outcome.is_none() {
            return Err(PluginError::Failed(
                "provider stream ended without a terminal run receipt".into(),
            ));
        }
        let snapshot = handle.lock().await.snapshot.clone();
        Ok(json!({
            "session_id": request.session_id,
            "snapshot": snapshot,
            "outcome": outcome,
        }))
    }

    async fn respond(&self, context: PluginContext, input: Value) -> Result<Value, PluginError> {
        let request: RespondRequest = decode(input)?;
        let handle = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| {
                PluginError::InvalidInput(format!("unknown session: {}", request.session_id))
            })?;
        let mut state = handle.lock().await;
        if context.project_id.as_deref() != Some(state.project_id.as_str()) {
            return Err(PluginError::InvalidInput(
                "session.respond project_id does not match the session checkout".into(),
            ));
        }
        let provider_session = state.session.id.clone();
        state
            .provider
            .respond(&provider_session, request.response)
            .await
            .map_err(provider_error)?;
        Ok(json!({"session_id": request.session_id, "accepted": true}))
    }

    async fn close(&self, input: Value) -> Result<Value, PluginError> {
        let request: CloseRequest = decode(input)?;
        let handle = self
            .sessions
            .lock()
            .await
            .remove(&request.session_id)
            .ok_or_else(|| {
                PluginError::InvalidInput(format!("unknown session: {}", request.session_id))
            })?;
        let close_result = {
            let mut state = handle.lock().await;
            let session_id = state.session.id.clone();
            state.provider.close_session(&session_id).await
        };
        if let Err(error) = close_result {
            self.sessions
                .lock()
                .await
                .insert(request.session_id.clone(), handle);
            return Err(provider_error(error));
        }
        Ok(json!({"session_id": request.session_id, "closed": true}))
    }
}

#[async_trait]
impl HeadlessPlugin for CodingPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        context: PluginContext,
        operation: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        match operation {
            SESSION_OPEN => self.open(context, input).await,
            SESSION_PROMPT => self.prompt(context, input).await,
            SESSION_RESPOND => self.respond(context, input).await,
            SESSION_CLOSE => self.close(input).await,
            _ => Err(PluginError::UnsupportedOperation {
                plugin: self.manifest.id.clone(),
                operation: operation.into(),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenRequest {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    options: SessionOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptRequest {
    session_id: String,
    input: PromptInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RespondRequest {
    session_id: String,
    response: ClientResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseRequest {
    session_id: String,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, PluginError> {
    serde_json::from_value(input).map_err(|error| PluginError::InvalidInput(error.to_string()))
}

fn portable_session_id(value: &str) -> Result<(), PluginError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(PluginError::InvalidInput(
            "session_id must be a bounded portable identifier".into(),
        ));
    }
    Ok(())
}

fn provider_error(error: AgentError) -> PluginError {
    PluginError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_requests_preserve_the_typed_provider_contract() {
        let open: OpenRequest = decode(json!({
            "session_id": "conversation-1",
            "options": {
                "cwd": "/srv/project",
                "mode": "auto",
                "collaboration_mode": "plan",
                "resume": { "items": [], "truncated": false }
            }
        }))
        .unwrap();
        assert_eq!(open.session_id.as_deref(), Some("conversation-1"));
        assert_eq!(open.options.cwd.as_deref(), Some("/srv/project"));
        assert_eq!(open.options.mode.as_deref(), Some("auto"));
        assert_eq!(
            open.options.collaboration_mode,
            Some(agent_core::CollaborationMode::Plan)
        );
        assert!(open.options.resume.is_some());

        let prompt: PromptRequest = decode(json!({
            "session_id": "conversation-1",
            "input": {
                "blocks": [{"type": "text", "text": "inspect the repository"}],
                "attachments": []
            }
        }))
        .unwrap();
        assert_eq!(prompt.session_id, "conversation-1");
        assert_eq!(prompt.input.blocks.len(), 1);
    }

    #[test]
    fn legacy_text_only_prompt_is_rejected() {
        let error = decode::<PromptRequest>(json!({
            "session_id": "conversation-1",
            "text": "this loses typed attachments"
        }))
        .unwrap_err();
        assert!(matches!(error, PluginError::InvalidInput(_)));
    }

    #[test]
    fn client_responses_keep_the_typed_boundary() {
        let request: RespondRequest = decode(json!({
            "session_id": "conversation-1",
            "response": {
                "kind": "permission",
                "request": "permission-1",
                "option": "allow-once"
            }
        }))
        .unwrap();
        assert_eq!(request.session_id, "conversation-1");
        assert!(matches!(
            request.response,
            ClientResponse::Permission { .. }
        ));
    }
}
